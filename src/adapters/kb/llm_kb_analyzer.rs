#![allow(clippy::manual_async_fn)] // Trait uses RPITIT pattern, impls must match

use std::future::Future;
use std::time::Duration;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use crate::domain::kb::KbAnalyzer;
use crate::domain::model::{KbAnalysis, KbAnalysisStatus};
use crate::infra::error::KbError;

// ---------------------------------------------------------------------------
// Prompt template
// ---------------------------------------------------------------------------

const KB_ANALYSIS_PROMPT: &str = r#"Analyze this repository. Respond ONLY with valid JSON, no markdown, no extra text.

Required JSON format:
{
  "what": "one sentence: what this repo does",
  "problem": "what real-world problem it solves",
  "architecture": "main architectural pattern used",
  "techniques": ["technique1", "technique2"],
  "steal": ["idea1", "idea2"],
  "uniqueness": "what makes it different from alternatives"
}

Rules:
- techniques: max 4 items, non-obvious techniques only
- steal: max 3 items, ideas applicable to other projects
- All values must be strings or arrays of strings
- No nested objects

Repository content:
{export_content}"#;

const KB_ANALYSIS_RETRY_SUFFIX: &str = r#"

Your previous response could not be parsed as JSON. Error: {parse_error}
Please respond with ONLY a valid JSON object — no markdown, no extra text, no code fences."#;

// ---------------------------------------------------------------------------
// Internal serde struct — LLM response shape
// ---------------------------------------------------------------------------

/// Raw LLM response, deserialized with liberal defaults so partial responses
/// don't fail at the serde layer (defense layer 3).
#[derive(Debug, Deserialize, Serialize, Default)]
struct LlmAnalysisOutput {
    #[serde(default)]
    what: String,
    #[serde(default)]
    problem: String,
    #[serde(default)]
    architecture: String,
    #[serde(default)]
    techniques: Vec<String>,
    #[serde(default)]
    steal: Vec<String>,
    #[serde(default)]
    uniqueness: String,
}

// ---------------------------------------------------------------------------
// OpenAI-compatible chat completion response shapes
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

#[derive(Debug, Deserialize)]
struct ChatMessage {
    content: String,
}

#[derive(Debug, Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<ChatRequestMessage<'a>>,
    response_format: ResponseFormat,
    temperature: f32,
}

#[derive(Debug, Serialize)]
struct ChatRequestMessage<'a> {
    role: &'a str,
    content: String,
}

#[derive(Debug, Serialize)]
struct ResponseFormat {
    #[serde(rename = "type")]
    kind: &'static str,
}

// ---------------------------------------------------------------------------
// LlmKbAnalyzer
// ---------------------------------------------------------------------------

/// Calls an OpenAI-compatible LLM to produce structured KB analysis.
///
/// Implements a 5-layer JSON defense strategy:
/// 1. `response_format: {"type": "json_object"}` instructs the LLM to emit JSON.
/// 2. Strip ` ```json ` / ` ``` ` markdown fences from the response text.
/// 3. All `LlmAnalysisOutput` fields use `#[serde(default)]` — partial responses parse.
/// 4. On parse failure, retry ONCE with an error-feedback suffix appended to the prompt.
/// 5. On second failure, return `KbAnalysis { status: ParseFailed, raw_llm_response }`.
///
/// Cheaply cloneable — `reqwest::Client` is internally `Arc`-backed.
#[derive(Clone)]
pub struct LlmKbAnalyzer {
    client: reqwest::Client,
    base_url: String,
    model: String,
    auth_token: Option<String>,
}

impl LlmKbAnalyzer {
    /// Create a new `LlmKbAnalyzer`.
    ///
    /// * `base_url` — e.g. `"http://localhost:3456"` (no trailing slash)
    /// * `model` — model name forwarded in the request body
    /// * `auth_token` — if `Some`, sent as `Authorization: Bearer <token>`
    pub fn new(base_url: String, model: String, auth_token: Option<String>) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .expect("failed to build reqwest client");
        Self {
            client,
            base_url,
            model,
            auth_token,
        }
    }

    /// Build the prompt by substituting `export_content` into the template.
    fn build_prompt(&self, export_content: &str) -> String {
        KB_ANALYSIS_PROMPT.replace("{export_content}", export_content)
    }

    /// Build a retry prompt with parse-error feedback appended.
    fn build_retry_prompt(&self, export_content: &str, parse_error: &str) -> String {
        let base = self.build_prompt(export_content);
        let suffix = KB_ANALYSIS_RETRY_SUFFIX.replace("{parse_error}", parse_error);
        format!("{base}{suffix}")
    }

    /// POST to the LLM completions endpoint with the given prompt.
    async fn post_prompt(
        &self,
        prompt: String,
        repo_label: &str,
    ) -> Result<String, KbError> {
        let url = format!("{}/v1/chat/completions", self.base_url);

        let body = ChatRequest {
            model: &self.model,
            messages: vec![ChatRequestMessage {
                role: "user",
                content: prompt,
            }],
            response_format: ResponseFormat { kind: "json_object" },
            temperature: 0.2,
        };

        let mut req = self.client.post(&url).json(&body);
        if let Some(token) = &self.auth_token {
            req = req.header("Authorization", format!("Bearer {token}"));
        }

        let response = req.send().await.map_err(|e| KbError::LlmRequest {
            repo: repo_label.to_owned(),
            reason: e.to_string(),
        })?;

        if !response.status().is_success() {
            let status = response.status();
            let body_text = response.text().await.unwrap_or_default();
            return Err(KbError::LlmRequest {
                repo: repo_label.to_owned(),
                reason: format!("HTTP {status}: {body_text}"),
            });
        }

        let completion: ChatCompletionResponse =
            response.json().await.map_err(|e| KbError::LlmRequest {
                repo: repo_label.to_owned(),
                reason: format!("failed to parse completion response: {e}"),
            })?;

        let content = completion
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .ok_or_else(|| KbError::LlmRequest {
                repo: repo_label.to_owned(),
                reason: "empty choices array in LLM response".to_owned(),
            })?;

        Ok(content)
    }

    /// Parse the raw LLM response content applying defense layers 2 & 3.
    ///
    /// Layer 2: strip markdown code fences before attempting parse.
    /// Layer 3: `#[serde(default)]` on all `LlmAnalysisOutput` fields.
    fn try_parse(raw: &str) -> Result<LlmAnalysisOutput, String> {
        let stripped = strip_code_fences(raw);
        serde_json::from_str::<LlmAnalysisOutput>(stripped.trim())
            .map_err(|e| e.to_string())
    }
}

// ---------------------------------------------------------------------------
// KbAnalyzer impl
// ---------------------------------------------------------------------------

impl KbAnalyzer for LlmKbAnalyzer {
    fn analyze(
        &self,
        repo_context: &str,
        owner: &str,
        repo_name: &str,
    ) -> impl Future<Output = Result<KbAnalysis, KbError>> + Send {
        let repo_label = format!("{owner}/{repo_name}");
        let prompt = self.build_prompt(repo_context);
        let repo_context = repo_context.to_owned();
        let owner = owner.to_owned();
        let repo_name = repo_name.to_owned();

        async move {
            // Layer 1: response_format in request (handled in post_prompt).
            let raw = self.post_prompt(prompt, &repo_label).await?;
            debug!("llm raw response for {repo_label}: {raw}");

            // Layers 2 & 3: fence stripping + serde defaults.
            match Self::try_parse(&raw) {
                Ok(output) => {
                    Ok(map_output_to_analysis(output, &owner, &repo_name))
                }
                Err(first_err) => {
                    // Layer 4: retry once with error feedback.
                    warn!(
                        "llm parse failed for {repo_label} (attempt 1): {first_err}; retrying"
                    );
                    let retry_prompt = self.build_retry_prompt(&repo_context, &first_err);
                    let raw2 = self.post_prompt(retry_prompt, &repo_label).await?;

                    match Self::try_parse(&raw2) {
                        Ok(output) => Ok(map_output_to_analysis(output, &owner, &repo_name)),
                        Err(second_err) => {
                            // Layer 5: store raw response, mark ParseFailed.
                            warn!(
                                "llm parse failed for {repo_label} (attempt 2): {second_err}; \
                                 storing raw response"
                            );
                            Ok(KbAnalysis {
                                owner,
                                repo_name,
                                status: KbAnalysisStatus::ParseFailed,
                                raw_llm_response: Some(raw2),
                                analyzed_at: None,
                                ..Default::default()
                            })
                        }
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Defense layer 2: strip ` ```json ` / ` ``` ` fences from LLM output.
fn strip_code_fences(s: &str) -> &str {
    let s = s.trim();

    // Try ```json ... ```
    if let Some(inner) = s
        .strip_prefix("```json")
        .or_else(|| s.strip_prefix("```JSON"))
    {
        if let Some(stripped) = inner.strip_suffix("```") {
            return stripped.trim();
        }
        // Fence opened but not closed — still strip the prefix and trim.
        return inner.trim();
    }

    // Try plain ``` ... ```
    if let Some(inner) = s.strip_prefix("```") {
        if let Some(stripped) = inner.strip_suffix("```") {
            return stripped.trim();
        }
        return inner.trim();
    }

    s
}

/// Map the parsed `LlmAnalysisOutput` to a full `KbAnalysis`.
/// Repo metadata fields are filled in by the caller (adapter or pipeline).
fn map_output_to_analysis(output: LlmAnalysisOutput, owner: &str, repo_name: &str) -> KbAnalysis {
    KbAnalysis {
        owner: owner.to_owned(),
        repo_name: repo_name.to_owned(),
        what: output.what,
        problem: output.problem,
        architecture: output.architecture,
        techniques: output.techniques,
        steal: output.steal,
        uniqueness: output.uniqueness,
        status: KbAnalysisStatus::Complete,
        analyzed_at: Some(Utc::now()),
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // ── Layer 2: fence stripping ─────────────────────────────────────────────

    #[test]
    fn strip_code_fences_json_fence() {
        let input = "```json\n{\"what\":\"hello\"}\n```";
        assert_eq!(strip_code_fences(input), "{\"what\":\"hello\"}");
    }

    #[test]
    fn strip_code_fences_plain_fence() {
        let input = "```\n{\"what\":\"hello\"}\n```";
        assert_eq!(strip_code_fences(input), "{\"what\":\"hello\"}");
    }

    #[test]
    fn strip_code_fences_no_fence() {
        let input = "{\"what\":\"hello\"}";
        assert_eq!(strip_code_fences(input), "{\"what\":\"hello\"}");
    }

    #[test]
    fn strip_code_fences_unclosed_fence() {
        let input = "```json\n{\"what\":\"hello\"}";
        assert_eq!(strip_code_fences(input), "{\"what\":\"hello\"}");
    }

    #[test]
    fn strip_code_fences_uppercase_json() {
        let input = "```JSON\n{\"what\":\"hello\"}\n```";
        assert_eq!(strip_code_fences(input), "{\"what\":\"hello\"}");
    }

    // ── Layer 3: serde defaults ──────────────────────────────────────────────

    #[test]
    fn try_parse_partial_json_uses_defaults() {
        // Only "what" field present — rest should default to empty.
        let json = r#"{"what": "a repo"}"#;
        let output = LlmKbAnalyzer::try_parse(json).unwrap();
        assert_eq!(output.what, "a repo");
        assert_eq!(output.problem, "");
        assert!(output.techniques.is_empty());
    }

    #[test]
    fn try_parse_full_json_round_trip() {
        let json = r#"{
            "what": "A tool",
            "problem": "Solves X",
            "architecture": "hexagonal",
            "techniques": ["async", "CQRS"],
            "steal": ["plugin system"],
            "uniqueness": "very fast"
        }"#;
        let output = LlmKbAnalyzer::try_parse(json).unwrap();
        assert_eq!(output.what, "A tool");
        assert_eq!(output.techniques, vec!["async", "CQRS"]);
        assert_eq!(output.steal, vec!["plugin system"]);
    }

    #[test]
    fn try_parse_with_json_fence_succeeds() {
        let json = "```json\n{\"what\": \"fenced\"}\n```";
        let output = LlmKbAnalyzer::try_parse(json).unwrap();
        assert_eq!(output.what, "fenced");
    }

    #[test]
    fn try_parse_invalid_json_returns_err() {
        let result = LlmKbAnalyzer::try_parse("not json at all");
        assert!(result.is_err());
    }

    #[test]
    fn try_parse_empty_object_all_defaults() {
        let output = LlmKbAnalyzer::try_parse("{}").unwrap();
        assert_eq!(output.what, "");
        assert!(output.techniques.is_empty());
        assert!(output.steal.is_empty());
    }

    // ── Layer 4 & 5: retry logic (wiremock) ─────────────────────────────────

    fn make_completion_body(content: &str) -> serde_json::Value {
        serde_json::json!({
            "choices": [{"message": {"content": content}}]
        })
    }

    #[tokio::test]
    async fn analyze_happy_path() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(make_completion_body(
                r#"{"what":"A tool","problem":"Solves X","architecture":"hexagonal","techniques":["async"],"steal":["plugins"],"uniqueness":"fast"}"#,
            )))
            .mount(&server)
            .await;

        let analyzer = LlmKbAnalyzer::new(server.uri(), "test-model".into(), None);
        let result = analyzer.analyze("some context", "owner", "repo").await.unwrap();
        assert_eq!(result.what, "A tool");
        assert_eq!(result.status, KbAnalysisStatus::Complete);
        assert!(result.raw_llm_response.is_none());
    }

    #[tokio::test]
    async fn analyze_first_response_invalid_retries_and_succeeds() {
        let server = MockServer::start().await;
        // First call: invalid JSON
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(make_completion_body(
                "not valid json",
            )))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        // Second call: valid JSON
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(make_completion_body(
                r#"{"what":"Retry success","problem":"","architecture":"","techniques":[],"steal":[],"uniqueness":""}"#,
            )))
            .mount(&server)
            .await;

        let analyzer = LlmKbAnalyzer::new(server.uri(), "test-model".into(), None);
        let result = analyzer.analyze("context", "owner", "repo").await.unwrap();
        assert_eq!(result.what, "Retry success");
        assert_eq!(result.status, KbAnalysisStatus::Complete);
    }

    #[tokio::test]
    async fn analyze_both_attempts_fail_returns_parse_failed() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(make_completion_body(
                "still not json",
            )))
            .mount(&server)
            .await;

        let analyzer = LlmKbAnalyzer::new(server.uri(), "test-model".into(), None);
        let result = analyzer.analyze("context", "owner", "repo").await.unwrap();
        assert_eq!(result.status, KbAnalysisStatus::ParseFailed);
        assert!(result.raw_llm_response.is_some());
        assert_eq!(result.owner, "owner");
        assert_eq!(result.repo_name, "repo");
    }

    #[tokio::test]
    async fn analyze_fenced_json_response_parses_correctly() {
        let server = MockServer::start().await;
        let fenced = "```json\n{\"what\":\"fenced tool\",\"problem\":\"\",\"architecture\":\"\",\"techniques\":[],\"steal\":[],\"uniqueness\":\"\"}\n```";
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(make_completion_body(fenced)),
            )
            .mount(&server)
            .await;

        let analyzer = LlmKbAnalyzer::new(server.uri(), "test-model".into(), None);
        let result = analyzer.analyze("context", "owner", "repo").await.unwrap();
        assert_eq!(result.what, "fenced tool");
        assert_eq!(result.status, KbAnalysisStatus::Complete);
    }

    #[tokio::test]
    async fn analyze_http_error_returns_llm_request_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(500).set_body_string("internal error"))
            .mount(&server)
            .await;

        let analyzer = LlmKbAnalyzer::new(server.uri(), "test-model".into(), None);
        let result = analyzer.analyze("context", "owner", "repo").await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, KbError::LlmRequest { .. }));
    }

    #[tokio::test]
    async fn analyze_sends_auth_header_when_token_provided() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(wiremock::matchers::header(
                "Authorization",
                "Bearer secret-token",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(make_completion_body(
                r#"{"what":"auth test"}"#,
            )))
            .mount(&server)
            .await;

        let analyzer = LlmKbAnalyzer::new(
            server.uri(),
            "test-model".into(),
            Some("secret-token".into()),
        );
        let result = analyzer.analyze("context", "owner", "repo").await.unwrap();
        assert_eq!(result.what, "auth test");
    }

    // ── Phase 6.2: named integration tests ──────────────────────────────────

    #[tokio::test]
    async fn llm_analyzer_happy_path() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(make_completion_body(
                r#"{"what":"Repo Scanner","problem":"Discover repos","architecture":"hexagonal","techniques":["async","pipeline"],"steal":["plugin system"],"uniqueness":"blazing fast"}"#,
            )))
            .mount(&server)
            .await;

        let analyzer = LlmKbAnalyzer::new(server.uri(), "test-model".into(), None);
        let result = analyzer.analyze("some export content", "owner", "repo").await.unwrap();

        assert_eq!(result.what, "Repo Scanner");
        assert_eq!(result.problem, "Discover repos");
        assert_eq!(result.architecture, "hexagonal");
        assert_eq!(result.techniques, vec!["async", "pipeline"]);
        assert_eq!(result.steal, vec!["plugin system"]);
        assert_eq!(result.uniqueness, "blazing fast");
        assert_eq!(result.status, KbAnalysisStatus::Complete);
        assert!(result.raw_llm_response.is_none());
        assert_eq!(result.owner, "owner");
        assert_eq!(result.repo_name, "repo");
    }

    #[tokio::test]
    async fn llm_analyzer_strips_markdown_fences() {
        let server = MockServer::start().await;
        let fenced_content = "```json\n{\"what\":\"fenced tool\",\"problem\":\"none\",\"architecture\":\"flat\",\"techniques\":[],\"steal\":[],\"uniqueness\":\"unique\"}\n```";
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                make_completion_body(fenced_content),
            ))
            .mount(&server)
            .await;

        let analyzer = LlmKbAnalyzer::new(server.uri(), "test-model".into(), None);
        let result = analyzer.analyze("context", "owner", "repo").await.unwrap();
        assert_eq!(result.what, "fenced tool");
        assert_eq!(result.status, KbAnalysisStatus::Complete);
    }

    #[tokio::test]
    async fn llm_analyzer_retries_on_invalid_json() {
        let server = MockServer::start().await;
        // First call returns invalid JSON
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(make_completion_body(
                "this is not json at all",
            )))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        // Second call returns valid JSON
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(make_completion_body(
                r#"{"what":"Retry worked","problem":"","architecture":"","techniques":[],"steal":[],"uniqueness":""}"#,
            )))
            .mount(&server)
            .await;

        let analyzer = LlmKbAnalyzer::new(server.uri(), "test-model".into(), None);
        let result = analyzer.analyze("context", "owner", "repo").await.unwrap();
        assert_eq!(result.what, "Retry worked");
        assert_eq!(result.status, KbAnalysisStatus::Complete);
    }

    #[tokio::test]
    async fn llm_analyzer_saves_raw_on_total_failure() {
        let server = MockServer::start().await;
        // Both calls return invalid JSON
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(make_completion_body(
                "not json at all",
            )))
            .mount(&server)
            .await;

        let analyzer = LlmKbAnalyzer::new(server.uri(), "test-model".into(), None);
        let result = analyzer.analyze("context", "owner", "repo").await.unwrap();

        assert_eq!(result.status, KbAnalysisStatus::ParseFailed);
        assert!(
            result.raw_llm_response.is_some(),
            "raw_llm_response must be Some on total failure"
        );
        assert_eq!(result.owner, "owner");
        assert_eq!(result.repo_name, "repo");
    }

    // ── Prompt builder ───────────────────────────────────────────────────────

    #[test]
    fn build_prompt_inserts_content() {
        let analyzer = LlmKbAnalyzer::new("http://localhost".into(), "m".into(), None);
        let prompt = analyzer.build_prompt("MY_CONTENT");
        assert!(prompt.contains("MY_CONTENT"));
        // Prompt instructs LLM to return JSON (case-insensitive check).
        assert!(
            prompt.to_lowercase().contains("json"),
            "prompt must mention JSON"
        );
    }

    #[test]
    fn build_retry_prompt_contains_error() {
        let analyzer = LlmKbAnalyzer::new("http://localhost".into(), "m".into(), None);
        let prompt = analyzer.build_retry_prompt("CONTENT", "unexpected token at line 1");
        assert!(prompt.contains("CONTENT"));
        assert!(prompt.contains("unexpected token at line 1"));
    }
}
