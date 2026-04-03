#![allow(clippy::manual_async_fn)] // Trait uses RPITIT pattern, impls must match

use std::future::Future;

use serde::Deserialize;
use tracing::{info, warn};
use url::Url;

use crate::domain::model::FeedEntry;
use crate::domain::source::Source;
use crate::infra::error::SourceError;

const REDDIT_BASE: &str = "https://www.reddit.com";
const USER_AGENT: &str = "repo-radar/0.1.0 (github.com/JNZader/repo-radar)";

/// Fetches posts from Reddit subreddits and extracts GitHub repo links.
pub struct RedditSource {
    subreddits: Vec<String>,
    limit: usize,
    client: reqwest::Client,
    /// Override base URL for testing.
    base_url: String,
}

impl RedditSource {
    #[must_use]
    pub fn new(subreddits: Vec<String>, limit: usize, client: reqwest::Client) -> Self {
        Self {
            subreddits,
            limit,
            client,
            base_url: REDDIT_BASE.to_string(),
        }
    }

    /// Create with a custom base URL (for testing with wiremock).
    #[cfg(test)]
    fn with_base_url(
        subreddits: Vec<String>,
        limit: usize,
        client: reqwest::Client,
        base_url: String,
    ) -> Self {
        Self {
            subreddits,
            limit,
            client,
            base_url,
        }
    }
}

/// Reddit listing response (simplified).
#[derive(Debug, Deserialize)]
struct RedditListing {
    data: RedditListingData,
}

#[derive(Debug, Deserialize)]
struct RedditListingData {
    children: Vec<RedditChild>,
}

#[derive(Debug, Deserialize)]
struct RedditChild {
    data: RedditPost,
}

#[derive(Debug, Deserialize)]
struct RedditPost {
    title: String,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    selftext: Option<String>,
    #[serde(default)]
    created_utc: Option<f64>,
    #[serde(default)]
    #[allow(dead_code)]
    subreddit: String,
}

impl Source for RedditSource {
    fn fetch(&self) -> impl Future<Output = Result<Vec<FeedEntry>, SourceError>> + Send {
        let subreddits = self.subreddits.clone();
        let limit = self.limit;
        let client = self.client.clone();
        let base_url = self.base_url.clone();
        async move { fetch_subreddits(&base_url, &subreddits, limit, &client).await }
    }

    fn name(&self) -> &'static str {
        "reddit"
    }
}

async fn fetch_subreddits(
    base_url: &str,
    subreddits: &[String],
    limit: usize,
    client: &reqwest::Client,
) -> Result<Vec<FeedEntry>, SourceError> {
    let mut all_entries = Vec::new();

    for subreddit in subreddits {
        let sub = subreddit.strip_prefix("r/").unwrap_or(subreddit);
        info!(subreddit = sub, "fetching Reddit posts");

        match fetch_single_subreddit(base_url, sub, limit, client).await {
            Ok(entries) => {
                info!(subreddit = sub, count = entries.len(), "Reddit entries found");
                all_entries.extend(entries);
            }
            Err(e) => {
                warn!(subreddit = sub, error = %e, "failed to fetch subreddit, skipping");
            }
        }
    }

    Ok(all_entries)
}

async fn fetch_single_subreddit(
    base_url: &str,
    subreddit: &str,
    limit: usize,
    client: &reqwest::Client,
) -> Result<Vec<FeedEntry>, SourceError> {
    let url = format!("{base_url}/r/{subreddit}/hot.json?limit={limit}");

    let response = client
        .get(&url)
        .header("User-Agent", USER_AGENT)
        .send()
        .await
        .map_err(|e| SourceError::FetchFailed {
            url: url.clone(),
            reason: e.to_string(),
        })?;

    let listing: RedditListing = response
        .json()
        .await
        .map_err(|e| SourceError::ParseFailed(format!("Reddit r/{subreddit}: {e}")))?;

    let mut entries = Vec::new();

    for child in listing.data.children {
        let post = &child.data;

        // Try to extract GitHub URL from post URL first
        if let Some(entry) = extract_github_entry_from_post(post) {
            entries.push(entry);
            continue;
        }

        // Fall back: scan selftext for GitHub URLs
        if let Some(ref selftext) = post.selftext {
            if let Some(github_url) = extract_github_url_from_text(selftext) {
                let published = post
                    .created_utc
                    .and_then(|t| chrono::DateTime::from_timestamp(t as i64, 0));

                entries.push(FeedEntry {
                    title: post.title.clone(),
                    repo_url: github_url,
                    description: Some(truncate_text(selftext, 200)),
                    published,
                    source_name: "reddit".to_string(),
                });
            }
        }
    }

    Ok(entries)
}

/// Try to create a `FeedEntry` if the post URL is a GitHub repo.
fn extract_github_entry_from_post(post: &RedditPost) -> Option<FeedEntry> {
    let url_str = post.url.as_deref()?;
    let url = Url::parse(url_str).ok()?;

    if url.host_str() != Some("github.com") {
        return None;
    }

    let segments: Vec<&str> = url
        .path_segments()?
        .filter(|s| !s.is_empty())
        .collect();

    if segments.len() < 2 {
        return None;
    }

    let repo_url =
        Url::parse(&format!("https://github.com/{}/{}", segments[0], segments[1])).ok()?;

    let published = post
        .created_utc
        .and_then(|t| chrono::DateTime::from_timestamp(t as i64, 0));

    Some(FeedEntry {
        title: post.title.clone(),
        repo_url,
        description: post.selftext.as_deref().map(|s| truncate_text(s, 200)),
        published,
        source_name: "reddit".to_string(),
    })
}

/// Extract the first GitHub repo URL found in a block of text.
fn extract_github_url_from_text(text: &str) -> Option<Url> {
    // Find all occurrences of "https://github.com/" in the text
    let needle = "https://github.com/";
    let mut search_from = 0;

    while let Some(start) = text[search_from..].find(needle) {
        let abs_start = search_from + start;
        // Find the end of the URL (whitespace, ), ], or end of string)
        let url_slice = &text[abs_start..];
        let end = url_slice
            .find(|c: char| c.is_whitespace() || c == ')' || c == ']' || c == '>' || c == '"')
            .unwrap_or(url_slice.len());
        let candidate = &url_slice[..end];

        if let Ok(url) = Url::parse(candidate) {
            let segments: Vec<&str> = url
                .path_segments()
                .into_iter()
                .flatten()
                .filter(|s| !s.is_empty())
                .collect();

            if segments.len() >= 2 {
                return Url::parse(&format!(
                    "https://github.com/{}/{}",
                    segments[0], segments[1]
                ))
                .ok();
            }
        }

        search_from = abs_start + needle.len();
    }

    None
}

fn truncate_text(text: &str, max_len: usize) -> String {
    if text.len() <= max_len {
        text.to_string()
    } else {
        format!("{}...", &text[..max_len])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn sample_reddit_listing() -> serde_json::Value {
        serde_json::json!({
            "data": {
                "children": [
                    {
                        "data": {
                            "title": "Check out my Rust CLI tool",
                            "url": "https://github.com/author1/rust-cli",
                            "selftext": "I built this cool thing",
                            "created_utc": 1700000000.0,
                            "subreddit": "rust"
                        }
                    },
                    {
                        "data": {
                            "title": "Interesting blog post",
                            "url": "https://blog.example.com/post",
                            "selftext": "Check https://github.com/author2/blog-tool for the code",
                            "created_utc": 1700000100.0,
                            "subreddit": "rust"
                        }
                    },
                    {
                        "data": {
                            "title": "Discussion about traits",
                            "url": null,
                            "selftext": "No GitHub links here, just text",
                            "created_utc": 1700000200.0,
                            "subreddit": "rust"
                        }
                    }
                ]
            }
        })
    }

    #[tokio::test]
    async fn fetches_subreddit_and_extracts_github_links() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/r/rust/hot.json"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(sample_reddit_listing()),
            )
            .mount(&server)
            .await;

        let source = RedditSource::with_base_url(
            vec!["rust".into()],
            25,
            reqwest::Client::new(),
            server.uri(),
        );
        let entries = source.fetch().await.unwrap();

        // Post 1: direct GitHub URL, Post 2: GitHub in selftext, Post 3: no GitHub
        assert_eq!(entries.len(), 2);

        assert_eq!(entries[0].title, "Check out my Rust CLI tool");
        assert_eq!(
            entries[0].repo_url.as_str(),
            "https://github.com/author1/rust-cli"
        );
        assert_eq!(entries[0].source_name, "reddit");
        assert!(entries[0].published.is_some());

        assert_eq!(entries[1].title, "Interesting blog post");
        assert_eq!(
            entries[1].repo_url.as_str(),
            "https://github.com/author2/blog-tool"
        );
    }

    #[tokio::test]
    async fn handles_multiple_subreddits() {
        let server = MockServer::start().await;

        let listing = serde_json::json!({
            "data": {
                "children": [{
                    "data": {
                        "title": "A post",
                        "url": "https://github.com/owner/repo",
                        "selftext": "",
                        "created_utc": 1700000000.0,
                        "subreddit": "programming"
                    }
                }]
            }
        });

        Mock::given(method("GET"))
            .and(path("/r/rust/hot.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&listing))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/r/programming/hot.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&listing))
            .mount(&server)
            .await;

        let source = RedditSource::with_base_url(
            vec!["rust".into(), "programming".into()],
            25,
            reqwest::Client::new(),
            server.uri(),
        );
        let entries = source.fetch().await.unwrap();

        assert_eq!(entries.len(), 2); // one from each subreddit
    }

    #[tokio::test]
    async fn strips_r_prefix_from_subreddit() {
        let server = MockServer::start().await;

        let listing = serde_json::json!({
            "data": {
                "children": [{
                    "data": {
                        "title": "Post",
                        "url": "https://github.com/a/b",
                        "selftext": "",
                        "created_utc": 1700000000.0,
                        "subreddit": "rust"
                    }
                }]
            }
        });

        Mock::given(method("GET"))
            .and(path("/r/rust/hot.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&listing))
            .mount(&server)
            .await;

        let source = RedditSource::with_base_url(
            vec!["r/rust".into()], // with r/ prefix
            25,
            reqwest::Client::new(),
            server.uri(),
        );
        let entries = source.fetch().await.unwrap();
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn extract_github_url_from_text_finds_url() {
        let text = "Check out https://github.com/owner/repo for details";
        let url = extract_github_url_from_text(text);
        assert_eq!(url.unwrap().as_str(), "https://github.com/owner/repo");
    }

    #[test]
    fn extract_github_url_from_text_handles_markdown() {
        let text = "See [here](https://github.com/owner/repo) for code";
        let url = extract_github_url_from_text(text);
        assert_eq!(url.unwrap().as_str(), "https://github.com/owner/repo");
    }

    #[test]
    fn extract_github_url_from_text_returns_none_without_github() {
        let text = "Check out https://example.com for details";
        assert!(extract_github_url_from_text(text).is_none());
    }

    #[test]
    fn truncate_text_within_limit() {
        assert_eq!(truncate_text("short", 100), "short");
    }

    #[test]
    fn truncate_text_exceeds_limit() {
        let result = truncate_text("a long text that exceeds", 10);
        assert_eq!(result, "a long tex...");
    }

    #[test]
    fn name_returns_reddit() {
        let source = RedditSource::new(vec![], 25, reqwest::Client::new());
        assert_eq!(source.name(), "reddit");
    }
}
