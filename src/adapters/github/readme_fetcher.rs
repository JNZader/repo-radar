//! GitHub API fetcher — retrieves repo metadata and README content without cloning.
//!
//! Uses the GitHub REST API (`/repos/{owner}/{repo}` and
//! `/repos/{owner}/{repo}/readme`) via plain `reqwest`.

use std::time::Duration;

use base64::Engine as _;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use tracing::debug;

use crate::infra::error::KbError;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Fetched GitHub repository metadata + README content.
///
/// The `context` field is a pre-formatted string (description + topics +
/// README) suitable for passing directly to `LlmKbAnalyzer::analyze()`.
#[derive(Debug, Clone)]
pub struct GithubRepoContext {
    pub owner: String,
    pub repo_name: String,
    pub pushed_at: Option<DateTime<Utc>>,
    /// Pre-formatted context string: description + topics + README text.
    /// Used as the `repo_context` argument to `KbAnalyzer::analyze`.
    pub context: String,
}

// ---------------------------------------------------------------------------
// GitHub API response shapes
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct RepoMetadata {
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    topics: Vec<String>,
    #[serde(default)]
    stargazers_count: i64,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    pushed_at: Option<String>,
    owner: OwnerField,
}

#[derive(Debug, Deserialize)]
struct OwnerField {
    login: String,
}

#[derive(Debug, Deserialize)]
struct ReadmeResponse {
    /// Base64-encoded README content.
    content: String,
}

// ---------------------------------------------------------------------------
// GithubReadmeFetcher
// ---------------------------------------------------------------------------

/// Fetches GitHub repo metadata and README via the GitHub REST API.
///
/// Cheaply cloneable — `reqwest::Client` is internally `Arc`-backed.
#[derive(Clone)]
pub struct GithubReadmeFetcher {
    client: reqwest::Client,
    token: Option<String>,
    /// Base URL for the GitHub API. Defaults to `https://api.github.com`.
    /// Overridable in tests via `new_with_base`.
    api_base: String,
}

impl GithubReadmeFetcher {
    /// Create a new fetcher targeting the real GitHub API.
    ///
    /// * `token` — GitHub personal access token (`Some("ghp_...")`). Strongly
    ///   recommended to avoid aggressive rate limiting. If `None`, unauthenticated
    ///   requests are made (60 req/hour limit).
    pub fn new(token: Option<String>) -> Self {
        Self::new_with_base(token, "https://api.github.com".to_owned())
    }

    /// Create a fetcher with a custom API base URL (used in tests with wiremock).
    pub fn new_with_base(token: Option<String>, api_base: String) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent("repo-radar/0.1")
            .build()
            .expect("failed to build reqwest client for GithubReadmeFetcher");
        Self {
            client,
            token,
            api_base,
        }
    }

    /// Parse a GitHub URL or `owner/repo` shorthand into `(owner, repo)`.
    ///
    /// Handles:
    /// - `https://github.com/owner/repo`
    /// - `https://github.com/owner/repo.git`
    /// - `owner/repo` shorthand
    pub fn parse_github_url(url: &str) -> Result<(String, String), KbError> {
        let url = url.trim().trim_end_matches('/');

        // Full GitHub URL
        if let Some(path) = url.strip_prefix("https://github.com/") {
            return Self::split_owner_repo(path, url);
        }
        if let Some(path) = url.strip_prefix("http://github.com/") {
            return Self::split_owner_repo(path, url);
        }

        // shorthand: "owner/repo" — exactly one slash, no protocol
        if url.contains('/') && !url.contains("://") {
            return Self::split_owner_repo(url, url);
        }

        Err(KbError::LlmRequest {
            repo: url.to_owned(),
            reason: format!("not a valid GitHub URL or owner/repo shorthand: {url}"),
        })
    }

    fn split_owner_repo(path: &str, original: &str) -> Result<(String, String), KbError> {
        let parts: Vec<&str> = path.splitn(3, '/').collect();
        if parts.len() < 2 || parts[0].is_empty() || parts[1].is_empty() {
            return Err(KbError::LlmRequest {
                repo: original.to_owned(),
                reason: format!("cannot extract owner/repo from: {original}"),
            });
        }
        let owner = parts[0].to_owned();
        let repo = parts[1].trim_end_matches(".git").to_owned();
        if repo.is_empty() {
            return Err(KbError::LlmRequest {
                repo: original.to_owned(),
                reason: format!("repo name is empty in: {original}"),
            });
        }
        Ok((owner, repo))
    }

    /// Fetch repo metadata and README for the given GitHub URL.
    ///
    /// * Parses `url` via `parse_github_url`.
    /// * Fetches `GET https://api.github.com/repos/{owner}/{repo}`.
    /// * Fetches README from `GET https://api.github.com/repos/{owner}/{repo}/readme`.
    ///   - On 404: no README — `context` will use description + topics only.
    ///   - On 429: returns `KbError::LlmRequest` with rate-limit message.
    pub async fn fetch(&self, url: &str) -> Result<GithubRepoContext, KbError> {
        let (owner, repo) = Self::parse_github_url(url)?;
        let repo_label = format!("{owner}/{repo}");

        // -- Fetch repo metadata ------------------------------------------------
        let meta = self.get_repo_metadata(&owner, &repo, &repo_label).await?;

        // -- Fetch README (404 is not an error) ---------------------------------
        let readme_text = self
            .get_readme(&owner, &repo, &repo_label)
            .await?;

        // -- Build context string -----------------------------------------------
        let context = build_context_string(&meta, readme_text.as_deref());

        // -- Parse pushed_at ----------------------------------------------------
        let pushed_at = meta
            .pushed_at
            .as_deref()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&Utc));

        debug!("fetched GitHub context for {repo_label}");

        Ok(GithubRepoContext {
            owner,
            repo_name: repo,
            pushed_at,
            context,
        })
    }

    // -------------------------------------------------------------------------
    // Private helpers
    // -------------------------------------------------------------------------

    fn add_auth_headers(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let req = req.header("Accept", "application/vnd.github.v3+json");
        if let Some(token) = &self.token {
            req.header("Authorization", format!("Bearer {token}"))
        } else {
            req
        }
    }

    async fn get_repo_metadata(
        &self,
        owner: &str,
        repo: &str,
        repo_label: &str,
    ) -> Result<RepoMetadata, KbError> {
        let url = format!("{}/repos/{owner}/{repo}", self.api_base);
        let req = self.add_auth_headers(self.client.get(&url));
        let resp = req.send().await.map_err(|e| KbError::LlmRequest {
            repo: repo_label.to_owned(),
            reason: format!("network error fetching repo metadata: {e}"),
        })?;

        let status = resp.status();
        if status.as_u16() == 429 {
            let retry_after = resp
                .headers()
                .get("Retry-After")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("unknown");
            return Err(KbError::LlmRequest {
                repo: repo_label.to_owned(),
                reason: format!(
                    "GitHub API rate-limited — set github_token in config (Retry-After: {retry_after}s)"
                ),
            });
        }
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(KbError::LlmRequest {
                repo: repo_label.to_owned(),
                reason: format!("GitHub API error {status}: {body}"),
            });
        }

        resp.json::<RepoMetadata>().await.map_err(|e| KbError::LlmRequest {
            repo: repo_label.to_owned(),
            reason: format!("failed to parse repo metadata: {e}"),
        })
    }

    /// Returns `Ok(Some(text))` on success, `Ok(None)` on 404.
    async fn get_readme(
        &self,
        owner: &str,
        repo: &str,
        repo_label: &str,
    ) -> Result<Option<String>, KbError> {
        let url = format!("{}/repos/{owner}/{repo}/readme", self.api_base);
        let req = self.add_auth_headers(self.client.get(&url));
        let resp = req.send().await.map_err(|e| KbError::LlmRequest {
            repo: repo_label.to_owned(),
            reason: format!("network error fetching README: {e}"),
        })?;

        let status = resp.status();

        if status.as_u16() == 404 {
            return Ok(None);
        }
        if status.as_u16() == 429 {
            let retry_after = resp
                .headers()
                .get("Retry-After")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("unknown");
            return Err(KbError::LlmRequest {
                repo: repo_label.to_owned(),
                reason: format!(
                    "GitHub API rate-limited — set github_token in config (Retry-After: {retry_after}s)"
                ),
            });
        }
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(KbError::LlmRequest {
                repo: repo_label.to_owned(),
                reason: format!("GitHub API error {status} on README: {body}"),
            });
        }

        let readme: ReadmeResponse = resp.json().await.map_err(|e| KbError::LlmRequest {
            repo: repo_label.to_owned(),
            reason: format!("failed to parse README response: {e}"),
        })?;

        // The `content` field from GitHub has newlines embedded in the base64.
        let cleaned = readme.content.replace('\n', "").replace('\r', "");
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(cleaned.as_bytes())
            .map_err(|e| KbError::LlmRequest {
                repo: repo_label.to_owned(),
                reason: format!("failed to base64-decode README: {e}"),
            })?;

        let text = String::from_utf8_lossy(&decoded).into_owned();
        Ok(Some(text))
    }
}

// ---------------------------------------------------------------------------
// Context builder
// ---------------------------------------------------------------------------

/// Build a single context string from repo metadata + optional README text.
///
/// Format mirrors what `repoforge export` produces: a human-readable block
/// that the LLM can analyze with `LlmKbAnalyzer`.
fn build_context_string(meta: &RepoMetadata, readme: Option<&str>) -> String {
    let mut parts: Vec<String> = Vec::new();

    parts.push(format!("Repository: {}/{}", meta.owner.login, meta.name));

    if let Some(desc) = &meta.description {
        if !desc.is_empty() {
            parts.push(format!("Description: {desc}"));
        }
    }

    if !meta.topics.is_empty() {
        parts.push(format!("Topics: {}", meta.topics.join(", ")));
    }

    if let Some(lang) = &meta.language {
        parts.push(format!("Primary language: {lang}"));
    }

    parts.push(format!("Stars: {}", meta.stargazers_count));

    if let Some(readme_text) = readme {
        if !readme_text.trim().is_empty() {
            parts.push(format!("\n--- README ---\n{readme_text}"));
        }
    }

    parts.join("\n")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // ── URL parsing ──────────────────────────────────────────────────────────

    #[test]
    fn parse_github_url_standard_url() {
        let (owner, repo) =
            GithubReadmeFetcher::parse_github_url("https://github.com/rust-lang/rust").unwrap();
        assert_eq!(owner, "rust-lang");
        assert_eq!(repo, "rust");
    }

    #[test]
    fn parse_github_url_trailing_slash() {
        let (owner, repo) =
            GithubReadmeFetcher::parse_github_url("https://github.com/owner/repo/").unwrap();
        assert_eq!(owner, "owner");
        assert_eq!(repo, "repo");
    }

    #[test]
    fn parse_github_url_dot_git_suffix() {
        let (owner, repo) =
            GithubReadmeFetcher::parse_github_url("https://github.com/owner/repo.git").unwrap();
        assert_eq!(owner, "owner");
        assert_eq!(repo, "repo");
    }

    #[test]
    fn parse_github_url_shorthand() {
        let (owner, repo) =
            GithubReadmeFetcher::parse_github_url("tokio-rs/tokio").unwrap();
        assert_eq!(owner, "tokio-rs");
        assert_eq!(repo, "tokio");
    }

    #[test]
    fn parse_github_url_invalid() {
        let result = GithubReadmeFetcher::parse_github_url("not-a-github-url-at-all");
        assert!(result.is_err(), "plain string without slash should fail");
    }

    #[test]
    fn parse_github_url_invalid_no_repo() {
        let result = GithubReadmeFetcher::parse_github_url("https://github.com/owner");
        assert!(result.is_err(), "URL with no repo segment should fail");
    }

    // ── Wiremock helpers ─────────────────────────────────────────────────────

    fn repo_meta_json() -> serde_json::Value {
        json!({
            "name": "my-repo",
            "description": "A test repository",
            "topics": ["rust", "cli"],
            "stargazers_count": 42,
            "language": "Rust",
            "pushed_at": "2025-06-01T12:00:00Z",
            "owner": { "login": "owner" }
        })
    }

    fn readme_json(content: &str) -> serde_json::Value {
        // base64-encode the content
        let encoded = base64::engine::general_purpose::STANDARD.encode(content);
        json!({ "content": encoded })
    }

    fn make_fetcher(base_url: &str) -> GithubReadmeFetcher {
        GithubReadmeFetcher::new_with_base(None, base_url.to_owned())
    }

    // ── Happy path ───────────────────────────────────────────────────────────

    #[tokio::test]
    async fn fetch_happy_path() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/repos/owner/my-repo"))
            .respond_with(ResponseTemplate::new(200).set_body_json(repo_meta_json()))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/repos/owner/my-repo/readme"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(readme_json("# Hello\nThis is a readme.")),
            )
            .mount(&server)
            .await;

        let fetcher = make_fetcher(&server.uri());
        let result = fetcher
            .fetch("https://github.com/owner/my-repo")
            .await
            .unwrap();

        assert_eq!(result.owner, "owner");
        assert_eq!(result.repo_name, "my-repo");
        assert!(result.pushed_at.is_some());
        assert!(result.context.contains("A test repository"));
        assert!(result.context.contains("rust"));
        assert!(result.context.contains("# Hello"));
    }

    // ── README 404 returns None — not an error ───────────────────────────────

    #[tokio::test]
    async fn fetch_readme_404_returns_none() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/repos/owner/no-readme"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "name": "no-readme",
                "description": "Repo without README",
                "topics": [],
                "stargazers_count": 0,
                "language": null,
                "pushed_at": "2025-01-01T00:00:00Z",
                "owner": { "login": "owner" }
            })))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/repos/owner/no-readme/readme"))
            .respond_with(ResponseTemplate::new(404).set_body_string("Not Found"))
            .mount(&server)
            .await;

        let fetcher = make_fetcher(&server.uri());
        let result = fetcher
            .fetch("https://github.com/owner/no-readme")
            .await
            .unwrap();

        assert_eq!(result.owner, "owner");
        // Context should NOT contain a README section
        assert!(!result.context.contains("--- README ---"));
        // Context should contain description
        assert!(result.context.contains("Repo without README"));
    }

    // ── Rate limit on metadata endpoint → error ───────────────────────────────

    #[tokio::test]
    async fn fetch_rate_limited_returns_error() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/repos/owner/my-repo"))
            .respond_with(
                ResponseTemplate::new(429)
                    .insert_header("Retry-After", "60")
                    .set_body_string("rate limited"),
            )
            .mount(&server)
            .await;

        let fetcher = make_fetcher(&server.uri());
        let result = fetcher.fetch("https://github.com/owner/my-repo").await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        match err {
            KbError::LlmRequest { reason, .. } => {
                assert!(
                    reason.contains("rate-limited"),
                    "error should mention rate-limiting; got: {reason}"
                );
            }
            other => panic!("expected KbError::LlmRequest, got: {other:?}"),
        }
    }

    // ── Rate limit on README endpoint → error ────────────────────────────────

    #[tokio::test]
    async fn fetch_readme_rate_limited_returns_error() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/repos/owner/my-repo"))
            .respond_with(ResponseTemplate::new(200).set_body_json(repo_meta_json()))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/repos/owner/my-repo/readme"))
            .respond_with(
                ResponseTemplate::new(429)
                    .insert_header("Retry-After", "30")
                    .set_body_string("rate limited"),
            )
            .mount(&server)
            .await;

        let fetcher = make_fetcher(&server.uri());
        let result = fetcher.fetch("https://github.com/owner/my-repo").await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, KbError::LlmRequest { .. }),
            "expected LlmRequest error"
        );
    }
}
