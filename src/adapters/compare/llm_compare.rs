#![allow(clippy::manual_async_fn)] // Trait uses RPITIT pattern, impls must match

use std::future::Future;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use crate::domain::compare::{CompareError, CompareResult, ComparedIdea, CompareService};
use crate::domain::model::KbAnalysis;

// ---------------------------------------------------------------------------
// Prompt template
// ---------------------------------------------------------------------------

const COMPARE_PROMPT_TEMPLATE: &str = r#"You know two repositories in detail.

SOURCE (external repo — the one you are studying for ideas):
What it does: {source_what}
Problem it solves: {source_problem}
Architecture: {source_architecture}
Techniques used: {source_techniques}
Interesting patterns: {source_steal}

TARGET (your own repo — where you want to apply the ideas):
What it does: {target_what}
Problem it solves: {target_problem}
Architecture: {target_architecture}
Current techniques: {target_techniques}

Generate specific, actionable ideas to improve TARGET based on what SOURCE does.
Respond ONLY with valid JSON, no markdown:
{
  "ideas": [
    {
      "title": "short imperative title",
      "description": "concrete: what to implement and how",
      "effort": "low",
      "impact": "high"
    }
  ]
}
Rules:
- Maximum 6 ideas
- Each idea must be concrete and specific to these two repos (not generic advice)
- effort and impact must be exactly: "low", "medium", or "high"
- Order by impact descending"#;

const COMPARE_RETRY_SUFFIX: &str = r#"

Your previous response could not be parsed as JSON. Error: {parse_error}
Please respond with ONLY a valid JSON object — no markdown, no extra text, no code fences."#;

// ---------------------------------------------------------------------------
// Internal serde structs — LLM response shape
// ---------------------------------------------------------------------------

/// Raw LLM idea item, deserialized with liberal defaults (defense layer 3).
#[derive(Debug, Deserialize, Serialize, Default)]
struct LlmIdeaItem {
    #[serde(default)]
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    effort: String,
    #[serde(default)]
    impact: String,
}

/// Raw LLM response container, deserialized with liberal defaults (defense layer 3).
#[derive(Debug, Deserialize, Serialize, Default)]
struct LlmCompareOutput {
    #[serde(default)]
    ideas: Vec<LlmIdeaItem>,
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
// LlmCompareService
// ---------------------------------------------------------------------------

/// Calls an OpenAI-compatible LLM to compare two repositories and generate ideas.
///
/// Implements a 5-layer JSON defense strategy:
/// 1. `response_format: {"type": "json_object"}` instructs the LLM to emit JSON.
/// 2. Strip ` ```json ` / ` ``` ` markdown fences from the response text.
/// 3. All `LlmCompareOutput` / `LlmIdeaItem` fields use `#[serde(default)]`.
/// 4. On parse failure, retry ONCE with an error-feedback suffix appended to the prompt.
/// 5. On second failure, return `CompareError::ParseFailed { raw }`.
///
/// Cheaply cloneable — `reqwest::Client` is internally `Arc`-backed.
#[derive(Clone)]
pub struct LlmCompareService {
    client: reqwest::Client,
    base_url: String,
    model: String,
    auth_token: Option<String>,
}

impl LlmCompareService {
    /// Create a new `LlmCompareService`.
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

    /// Build the prompt by substituting source and target `KbAnalysis` fields.
    fn build_prompt(&self, source: &KbAnalysis, target: &KbAnalysis) -> String {
        COMPARE_PROMPT_TEMPLATE
            .replace("{source_what}", &source.what)
            .replace("{source_problem}", &source.problem)
            .replace("{source_architecture}", &source.architecture)
            .replace("{source_techniques}", &source.techniques.join(", "))
            .replace("{source_steal}", &source.steal.join(", "))
            .replace("{target_what}", &target.what)
            .replace("{target_problem}", &target.problem)
            .replace("{target_architecture}", &target.architecture)
            .replace("{target_techniques}", &target.techniques.join(", "))
    }

    /// Build a retry prompt with parse-error feedback appended.
    fn build_retry_prompt(
        &self,
        source: &KbAnalysis,
        target: &KbAnalysis,
        parse_error: &str,
    ) -> String {
        let base = self.build_prompt(source, target);
        let suffix = COMPARE_RETRY_SUFFIX.replace("{parse_error}", parse_error);
        format!("{base}{suffix}")
    }

    /// POST to the LLM completions endpoint with the given prompt.
    async fn post_prompt(&self, prompt: String) -> Result<String, CompareError> {
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

        let response = req
            .send()
            .await
            .map_err(|e| CompareError::LlmError(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let body_text = response.text().await.unwrap_or_default();
            return Err(CompareError::LlmError(format!(
                "HTTP {status}: {body_text}"
            )));
        }

        let completion: ChatCompletionResponse = response
            .json()
            .await
            .map_err(|e| CompareError::LlmError(format!("failed to parse completion response: {e}")))?;

        let content = completion
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .ok_or_else(|| {
                CompareError::LlmError("empty choices array in LLM response".to_owned())
            })?;

        Ok(content)
    }

    /// Parse the raw LLM response applying defense layers 2 & 3.
    ///
    /// Layer 2: strip markdown code fences before attempting parse.
    /// Layer 3: `#[serde(default)]` on all output fields.
    fn try_parse(raw: &str) -> Result<LlmCompareOutput, String> {
        let stripped = strip_code_fences(raw);
        serde_json::from_str::<LlmCompareOutput>(stripped.trim()).map_err(|e| e.to_string())
    }

    /// Map `LlmCompareOutput` to a `CompareResult`.
    fn map_output(
        output: LlmCompareOutput,
        source: &KbAnalysis,
        target: &KbAnalysis,
    ) -> CompareResult {
        CompareResult {
            source_id: source.owner_repo_id(),
            target_id: target.owner_repo_id(),
            ideas: output
                .ideas
                .into_iter()
                .map(|item| ComparedIdea {
                    title: item.title,
                    description: item.description,
                    effort: item.effort,
                    impact: item.impact,
                })
                .collect(),
        }
    }
}

// ---------------------------------------------------------------------------
// CompareService impl
// ---------------------------------------------------------------------------

impl CompareService for LlmCompareService {
    fn compare(
        &self,
        source: &KbAnalysis,
        target: &KbAnalysis,
    ) -> impl Future<Output = Result<CompareResult, CompareError>> + Send {
        let prompt = self.build_prompt(source, target);
        let source = source.clone();
        let target = target.clone();

        async move {
            // Layer 1: response_format in request (handled in post_prompt).
            let raw = self.post_prompt(prompt).await?;
            debug!("llm raw compare response: {raw}");

            // Layers 2 & 3: fence stripping + serde defaults.
            match Self::try_parse(&raw) {
                Ok(output) => Ok(Self::map_output(output, &source, &target)),
                Err(first_err) => {
                    // Layer 4: retry once with error feedback.
                    warn!("compare parse failed (attempt 1): {first_err}; retrying");
                    let retry_prompt = self.build_retry_prompt(&source, &target, &first_err);
                    let raw2 = self.post_prompt(retry_prompt).await?;

                    match Self::try_parse(&raw2) {
                        Ok(output) => Ok(Self::map_output(output, &source, &target)),
                        Err(second_err) => {
                            // Layer 5: return ParseFailed with raw response.
                            warn!(
                                "compare parse failed (attempt 2): {second_err}; returning ParseFailed"
                            );
                            Err(CompareError::ParseFailed { raw: raw2 })
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn make_completion_body(content: &str) -> serde_json::Value {
        serde_json::json!({
            "choices": [{"message": {"content": content}}]
        })
    }

    fn make_kb_analysis(owner: &str, repo: &str) -> KbAnalysis {
        KbAnalysis {
            owner: owner.to_owned(),
            repo_name: repo.to_owned(),
            what: format!("{repo} does stuff"),
            problem: "solves a problem".to_owned(),
            architecture: "hexagonal".to_owned(),
            techniques: vec!["async".to_owned(), "cqrs".to_owned()],
            steal: vec!["plugin system".to_owned()],
            ..Default::default()
        }
    }

    fn valid_ideas_json(n: usize) -> String {
        let ideas: Vec<serde_json::Value> = (0..n)
            .map(|i| {
                serde_json::json!({
                    "title": format!("Idea {i}"),
                    "description": format!("Do thing {i}"),
                    "effort": "low",
                    "impact": "high"
                })
            })
            .collect();
        serde_json::json!({ "ideas": ideas }).to_string()
    }

    // ── compare_happy_path ───────────────────────────────────────────────────

    #[tokio::test]
    async fn compare_happy_path() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(make_completion_body(
                &valid_ideas_json(3),
            )))
            .mount(&server)
            .await;

        let service = LlmCompareService::new(server.uri(), "test-model".into(), None);
        let source = make_kb_analysis("owner", "source-repo");
        let target = make_kb_analysis("me", "target-repo");

        let result = service.compare(&source, &target).await.unwrap();

        assert_eq!(result.source_id, "owner/source-repo");
        assert_eq!(result.target_id, "me/target-repo");
        assert_eq!(result.ideas.len(), 3);
        assert_eq!(result.ideas[0].title, "Idea 0");
        assert_eq!(result.ideas[0].effort, "low");
        assert_eq!(result.ideas[0].impact, "high");
    }

    // ── compare_strips_markdown_fences ───────────────────────────────────────

    #[tokio::test]
    async fn compare_strips_markdown_fences() {
        let server = MockServer::start().await;
        let fenced = format!("```json\n{}\n```", valid_ideas_json(2));
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(make_completion_body(&fenced)))
            .mount(&server)
            .await;

        let service = LlmCompareService::new(server.uri(), "test-model".into(), None);
        let source = make_kb_analysis("owner", "source-repo");
        let target = make_kb_analysis("me", "target-repo");

        let result = service.compare(&source, &target).await.unwrap();

        assert_eq!(result.ideas.len(), 2);
        assert_eq!(result.ideas[0].title, "Idea 0");
    }

    // ── compare_retries_on_invalid_json ──────────────────────────────────────

    #[tokio::test]
    async fn compare_retries_on_invalid_json() {
        let server = MockServer::start().await;

        // First call: invalid JSON
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(make_completion_body(
                "not valid json at all",
            )))
            .up_to_n_times(1)
            .mount(&server)
            .await;

        // Second call: valid JSON
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(make_completion_body(
                &valid_ideas_json(1),
            )))
            .mount(&server)
            .await;

        let service = LlmCompareService::new(server.uri(), "test-model".into(), None);
        let source = make_kb_analysis("owner", "source-repo");
        let target = make_kb_analysis("me", "target-repo");

        let result = service.compare(&source, &target).await.unwrap();

        assert_eq!(result.ideas.len(), 1);
        assert_eq!(result.ideas[0].title, "Idea 0");
    }

    // ── compare_returns_parse_failed_on_double_failure ────────────────────────

    #[tokio::test]
    async fn compare_returns_parse_failed_on_double_failure() {
        let server = MockServer::start().await;

        // Both calls return invalid JSON
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(make_completion_body(
                "still not json",
            )))
            .mount(&server)
            .await;

        let service = LlmCompareService::new(server.uri(), "test-model".into(), None);
        let source = make_kb_analysis("owner", "source-repo");
        let target = make_kb_analysis("me", "target-repo");

        let err = service.compare(&source, &target).await.unwrap_err();

        assert!(
            matches!(err, CompareError::ParseFailed { ref raw } if !raw.is_empty()),
            "expected ParseFailed with non-empty raw, got: {err:?}"
        );
    }
}
