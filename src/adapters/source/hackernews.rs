#![allow(clippy::manual_async_fn)] // Trait uses RPITIT pattern, impls must match

use std::future::Future;

use serde::Deserialize;
use tracing::{info, warn};
use url::Url;

use crate::domain::model::FeedEntry;
use crate::domain::source::Source;
use crate::infra::error::SourceError;

const HN_API_BASE: &str = "https://hacker-news.firebaseio.com/v0";

/// Fetches "Show HN" stories from the HackerNews API that link to GitHub repos.
pub struct HackerNewsSource {
    limit: usize,
    client: reqwest::Client,
    /// Override base URL for testing.
    api_base: String,
}

impl HackerNewsSource {
    #[must_use]
    pub fn new(limit: usize, client: reqwest::Client) -> Self {
        Self {
            limit,
            client,
            api_base: HN_API_BASE.to_string(),
        }
    }

    /// Create with a custom API base URL (for testing with wiremock).
    #[cfg(test)]
    fn with_api_base(limit: usize, client: reqwest::Client, api_base: String) -> Self {
        Self {
            limit,
            client,
            api_base,
        }
    }
}

/// Minimal HN story item.
#[derive(Debug, Deserialize)]
struct HnItem {
    #[serde(default)]
    title: String,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    time: Option<i64>,
}

impl Source for HackerNewsSource {
    fn fetch(&self) -> impl Future<Output = Result<Vec<FeedEntry>, SourceError>> + Send {
        let limit = self.limit;
        let client = self.client.clone();
        let api_base = self.api_base.clone();
        async move { fetch_show_hn(&api_base, limit, &client).await }
    }

    fn name(&self) -> &'static str {
        "hackernews"
    }
}

async fn fetch_show_hn(
    api_base: &str,
    limit: usize,
    client: &reqwest::Client,
) -> Result<Vec<FeedEntry>, SourceError> {
    info!(limit = limit, "fetching HN Show stories");

    let show_url = format!("{api_base}/showstories.json");
    let story_ids: Vec<u64> = client
        .get(&show_url)
        .send()
        .await
        .map_err(|e| SourceError::FetchFailed {
            url: show_url.clone(),
            reason: e.to_string(),
        })?
        .json()
        .await
        .map_err(|e| SourceError::ParseFailed(format!("HN showstories: {e}")))?;

    let ids_to_fetch = &story_ids[..story_ids.len().min(limit)];
    info!(count = ids_to_fetch.len(), "fetching HN story details");

    let mut entries = Vec::new();

    for &id in ids_to_fetch {
        let item_url = format!("{api_base}/item/{id}.json");
        match client.get(&item_url).send().await {
            Ok(resp) => match resp.json::<HnItem>().await {
                Ok(item) => {
                    if let Some(entry) = hn_item_to_feed_entry(&item) {
                        entries.push(entry);
                    }
                }
                Err(e) => {
                    warn!(id = id, error = %e, "failed to parse HN item, skipping");
                }
            },
            Err(e) => {
                warn!(id = id, error = %e, "failed to fetch HN item, skipping");
            }
        }
    }

    info!(count = entries.len(), "HN entries with GitHub links");
    Ok(entries)
}

/// Convert an HN item to a `FeedEntry` if it links to a GitHub repo.
fn hn_item_to_feed_entry(item: &HnItem) -> Option<FeedEntry> {
    let url_str = item.url.as_deref()?;
    let url = Url::parse(url_str).ok()?;

    // Must be a GitHub repo URL
    if url.host_str() != Some("github.com") {
        return None;
    }

    // Must have at least owner/repo path segments
    let segments: Vec<&str> = url
        .path_segments()?
        .filter(|s| !s.is_empty())
        .collect();
    if segments.len() < 2 {
        return None;
    }

    // Normalize to just owner/repo
    let repo_url =
        Url::parse(&format!("https://github.com/{}/{}", segments[0], segments[1])).ok()?;

    let published = item
        .time
        .and_then(|t| chrono::DateTime::from_timestamp(t, 0));

    Some(FeedEntry {
        title: item.title.clone(),
        repo_url,
        description: None,
        published,
        source_name: "hackernews".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn fetches_show_hn_with_github_links() {
        let server = MockServer::start().await;

        // Mock showstories endpoint
        Mock::given(method("GET"))
            .and(path("/showstories.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(vec![1u64, 2, 3]))
            .mount(&server)
            .await;

        // Story 1: has GitHub URL
        Mock::given(method("GET"))
            .and(path("/item/1.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "title": "Show HN: My Rust tool",
                "url": "https://github.com/owner1/cool-tool",
                "time": 1700000000
            })))
            .mount(&server)
            .await;

        // Story 2: non-GitHub URL (should be skipped)
        Mock::given(method("GET"))
            .and(path("/item/2.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "title": "Show HN: My Website",
                "url": "https://example.com/product",
                "time": 1700000100
            })))
            .mount(&server)
            .await;

        // Story 3: GitHub URL with extra path
        Mock::given(method("GET"))
            .and(path("/item/3.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "title": "Show HN: Another tool",
                "url": "https://github.com/owner2/another/tree/main",
                "time": 1700000200
            })))
            .mount(&server)
            .await;

        let source = HackerNewsSource::with_api_base(
            10,
            reqwest::Client::new(),
            server.uri(),
        );
        let entries = source.fetch().await.unwrap();

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].title, "Show HN: My Rust tool");
        assert_eq!(
            entries[0].repo_url.as_str(),
            "https://github.com/owner1/cool-tool"
        );
        assert_eq!(entries[0].source_name, "hackernews");
        assert!(entries[0].published.is_some());

        // Story 3: URL normalized to owner/repo
        assert_eq!(
            entries[1].repo_url.as_str(),
            "https://github.com/owner2/another"
        );
    }

    #[tokio::test]
    async fn handles_empty_show_stories() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/showstories.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(Vec::<u64>::new()))
            .mount(&server)
            .await;

        let source = HackerNewsSource::with_api_base(
            10,
            reqwest::Client::new(),
            server.uri(),
        );
        let entries = source.fetch().await.unwrap();
        assert!(entries.is_empty());
    }

    #[tokio::test]
    async fn respects_limit() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/showstories.json"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(vec![1u64, 2, 3, 4, 5]),
            )
            .mount(&server)
            .await;

        for id in 1..=2 {
            Mock::given(method("GET"))
                .and(path(format!("/item/{id}.json")))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "title": format!("Story {id}"),
                    "url": format!("https://github.com/owner/repo-{id}"),
                    "time": 1700000000
                })))
                .mount(&server)
                .await;
        }

        let source = HackerNewsSource::with_api_base(
            2, // limit to 2
            reqwest::Client::new(),
            server.uri(),
        );
        let entries = source.fetch().await.unwrap();
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn hn_item_without_url_returns_none() {
        let item = HnItem {
            title: "No URL".into(),
            url: None,
            time: None,
        };
        assert!(hn_item_to_feed_entry(&item).is_none());
    }

    #[test]
    fn hn_item_non_github_url_returns_none() {
        let item = HnItem {
            title: "Not GitHub".into(),
            url: Some("https://example.com/tool".into()),
            time: None,
        };
        assert!(hn_item_to_feed_entry(&item).is_none());
    }

    #[test]
    fn hn_item_github_user_only_returns_none() {
        let item = HnItem {
            title: "User page".into(),
            url: Some("https://github.com/owner".into()),
            time: None,
        };
        assert!(hn_item_to_feed_entry(&item).is_none());
    }

    #[test]
    fn name_returns_hackernews() {
        let source = HackerNewsSource::new(10, reqwest::Client::new());
        assert_eq!(source.name(), "hackernews");
    }
}
