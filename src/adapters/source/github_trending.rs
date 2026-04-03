#![allow(clippy::manual_async_fn)] // Trait uses RPITIT pattern, impls must match

use std::future::Future;

use tracing::{info, warn};
use url::Url;

use crate::domain::model::FeedEntry;
use crate::domain::source::Source;
use crate::infra::error::SourceError;

/// Fetches trending repositories from GitHub's trending page via HTML scraping.
pub struct GitHubTrendingSource {
    language: Option<String>,
    since: String,
    client: reqwest::Client,
}

impl GitHubTrendingSource {
    #[must_use]
    pub fn new(language: Option<String>, since: String, client: reqwest::Client) -> Self {
        Self {
            language,
            since,
            client,
        }
    }

    fn trending_url(&self) -> String {
        let mut url = "https://github.com/trending".to_string();
        if let Some(ref lang) = self.language {
            url.push('/');
            url.push_str(&lang.to_lowercase());
        }
        url.push_str("?since=");
        url.push_str(&self.since);
        url
    }
}

impl Source for GitHubTrendingSource {
    fn fetch(&self) -> impl Future<Output = Result<Vec<FeedEntry>, SourceError>> + Send {
        let url = self.trending_url();
        let client = self.client.clone();
        let language = self.language.clone();
        async move { fetch_trending(&url, &client, language.as_deref()).await }
    }

    fn name(&self) -> &'static str {
        "github-trending"
    }
}

async fn fetch_trending(
    url: &str,
    client: &reqwest::Client,
    language: Option<&str>,
) -> Result<Vec<FeedEntry>, SourceError> {
    info!(url = url, "fetching GitHub trending");

    let response = client
        .get(url)
        .header("Accept", "text/html")
        .send()
        .await
        .map_err(|e| SourceError::FetchFailed {
            url: url.to_string(),
            reason: e.to_string(),
        })?;

    let body = response
        .text()
        .await
        .map_err(|e| SourceError::FetchFailed {
            url: url.to_string(),
            reason: e.to_string(),
        })?;

    let entries = parse_trending_html(&body, language);
    info!(count = entries.len(), "parsed trending repos");
    Ok(entries)
}

/// Parse GitHub trending HTML to extract repo links.
///
/// The trending page contains `<h2>` elements with `<a>` tags whose href
/// follows the pattern `/{owner}/{repo}`. We look for links inside elements
/// with the class `h3` or within article tags that contain repo paths.
fn parse_trending_html(html: &str, _language: Option<&str>) -> Vec<FeedEntry> {
    let mut entries = Vec::new();

    // GitHub trending page has repo links in the pattern:
    //   <a href="/owner/repo" ...>
    // inside article elements. We find all href="/owner/repo" patterns.
    for line in html.lines() {
        // Look for the specific trending repo link pattern
        if let Some(entry) = extract_trending_repo_from_line(line) {
            // Avoid duplicates by URL
            if !entries.iter().any(|e: &FeedEntry| e.repo_url == entry.repo_url) {
                entries.push(entry);
            }
        }
    }

    entries
}

/// Extract a trending repo entry from an HTML line containing an href like `/owner/repo`.
fn extract_trending_repo_from_line(line: &str) -> Option<FeedEntry> {
    // Pattern: href="/owner/repo" inside an <h2> or similar heading element
    // The trending page uses: <h2 class="h3 ..."><a href="/owner/repo" ...>
    let trimmed = line.trim();
    if !trimmed.contains("href=\"/") {
        return None;
    }

    // Only consider lines that look like repo heading links (contain class="h3" or similar)
    // or have the Box-row pattern used in GitHub trending
    let is_repo_link = trimmed.contains("h3")
        || trimmed.contains("repo-")
        || (trimmed.starts_with("<a") && trimmed.contains("color-fg-default"));

    if !is_repo_link {
        return None;
    }

    // Extract href value
    let href_start = trimmed.find("href=\"/")?;
    let path_start = href_start + 6; // skip `href="`
    let path = &trimmed[path_start..];
    let path_end = path.find('"')?;
    let path = &path[..path_end];

    // Must be /owner/repo (exactly 2 segments)
    let segments: Vec<&str> = path.trim_matches('/').split('/').collect();
    if segments.len() != 2 || segments[0].is_empty() || segments[1].is_empty() {
        return None;
    }

    let owner = segments[0];
    let repo = segments[1];
    let repo_url = Url::parse(&format!("https://github.com/{owner}/{repo}")).ok()?;

    Some(FeedEntry {
        title: format!("{owner}/{repo}"),
        repo_url,
        description: None,
        published: None,
        source_name: "github-trending".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn sample_trending_html(base_url: &str) -> String {
        format!(
            r#"<html>
<body>
<div class="Box">
  <article class="Box-row">
    <h2 class="h3 lh-condensed">
      <a href="/tokio-rs/tokio" class="color-fg-default" data-hydro-click>
        <span>tokio-rs</span> /
        <span class="fw-semibold">tokio</span>
      </a>
    </h2>
    <p class="col-9 color-fg-muted">An async runtime for Rust</p>
  </article>
  <article class="Box-row">
    <h2 class="h3 lh-condensed">
      <a href="/rust-lang/rust" class="color-fg-default" data-hydro-click>
        <span>rust-lang</span> /
        <span class="fw-semibold">rust</span>
      </a>
    </h2>
    <p class="col-9 color-fg-muted">The Rust programming language</p>
  </article>
  <article class="Box-row">
    <h2 class="h3 lh-condensed">
      <a href="/serde-rs/serde" class="color-fg-default" data-hydro-click>
        <span>serde-rs</span> /
        <span class="fw-semibold">serde</span>
      </a>
    </h2>
  </article>
</div>
</body>
</html>"#
        )
    }

    #[tokio::test]
    async fn fetches_trending_and_returns_entries() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/trending"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(sample_trending_html(&server.uri())),
            )
            .mount(&server)
            .await;

        // Create source pointing at mock server
        let client = reqwest::Client::new();
        let url = format!("{}/trending?since=daily", server.uri());
        let entries = fetch_trending(&url, &client, None).await.unwrap();

        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].title, "tokio-rs/tokio");
        assert_eq!(
            entries[0].repo_url.as_str(),
            "https://github.com/tokio-rs/tokio"
        );
        assert_eq!(entries[0].source_name, "github-trending");
        assert_eq!(entries[1].title, "rust-lang/rust");
        assert_eq!(entries[2].title, "serde-rs/serde");
    }

    #[tokio::test]
    async fn handles_fetch_failure() {
        let server = MockServer::start().await;
        // No mocks mounted — server returns 404

        let client = reqwest::Client::new();
        let url = format!("{}/trending?since=daily", server.uri());
        let result = fetch_trending(&url, &client, None).await;

        // Should still succeed with empty or parse what it gets
        assert!(result.is_ok());
    }

    #[test]
    fn parses_empty_html() {
        let entries = parse_trending_html("<html><body></body></html>", None);
        assert!(entries.is_empty());
    }

    #[test]
    fn deduplicates_repo_entries() {
        let html = r#"
        <h2 class="h3"><a href="/owner/repo" class="color-fg-default">owner/repo</a></h2>
        <h2 class="h3"><a href="/owner/repo" class="color-fg-default">owner/repo</a></h2>
        "#;
        let entries = parse_trending_html(html, None);
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn ignores_non_repo_links() {
        let html = r#"
        <a href="/about" class="h3">About</a>
        <a href="/owner/repo/issues" class="h3">Issues</a>
        "#;
        let entries = parse_trending_html(html, None);
        // /about has 1 segment, /owner/repo/issues has 3 — both skipped
        assert!(entries.is_empty());
    }

    #[test]
    fn trending_url_construction() {
        let source = GitHubTrendingSource::new(None, "daily".into(), reqwest::Client::new());
        assert_eq!(source.trending_url(), "https://github.com/trending?since=daily");

        let source = GitHubTrendingSource::new(
            Some("rust".into()),
            "weekly".into(),
            reqwest::Client::new(),
        );
        assert_eq!(
            source.trending_url(),
            "https://github.com/trending/rust?since=weekly"
        );
    }

    #[test]
    fn name_returns_github_trending() {
        let source = GitHubTrendingSource::new(None, "daily".into(), reqwest::Client::new());
        assert_eq!(source.name(), "github-trending");
    }
}
