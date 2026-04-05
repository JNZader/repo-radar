#![allow(clippy::manual_async_fn)] // Trait uses RPITIT pattern, impls must match

use std::future::Future;

use tracing::{info, warn};
use url::Url;

use crate::config::FeedConfig;
use crate::domain::model::FeedEntry;
use crate::domain::source::Source;
use crate::infra::error::SourceError;

/// Fetches entries from one or more RSS/Atom feeds and extracts GitHub repo URLs.
pub struct RssSource {
    feeds: Vec<FeedConfig>,
    client: reqwest::Client,
}

impl RssSource {
    /// Create a new `RssSource` with the given feed configurations and HTTP client.
    #[must_use]
    pub fn new(feeds: Vec<FeedConfig>, client: reqwest::Client) -> Self {
        Self { feeds, client }
    }
}

impl Source for RssSource {
    fn fetch(&self) -> impl Future<Output = Result<Vec<FeedEntry>, SourceError>> + Send {
        let feeds = self.feeds.clone();
        let client = self.client.clone();
        async move { fetch_all_feeds(&feeds, &client).await }
    }

    fn name(&self) -> &'static str {
        "rss"
    }
}

async fn fetch_all_feeds(
    feeds: &[FeedConfig],
    client: &reqwest::Client,
) -> Result<Vec<FeedEntry>, SourceError> {
    let mut all_entries = Vec::new();

    for feed_config in feeds {
        let feed_name = feed_config
            .name
            .as_deref()
            .unwrap_or(feed_config.url.as_str());

        info!(feed = feed_name, "fetching feed");

        match fetch_single_feed(feed_config, client).await {
            Ok(entries) => {
                info!(feed = feed_name, count = entries.len(), "parsed entries");
                all_entries.extend(entries);
            }
            Err(e) => {
                warn!(feed = feed_name, error = %e, "failed to fetch feed, skipping");
            }
        }
    }

    Ok(all_entries)
}

async fn fetch_single_feed(
    feed_config: &FeedConfig,
    client: &reqwest::Client,
) -> Result<Vec<FeedEntry>, SourceError> {
    let response = client
        .get(&feed_config.url)
        .send()
        .await
        .map_err(|e| SourceError::FetchFailed {
            url: feed_config.url.clone(),
            reason: e.to_string(),
        })?;

    let bytes = response
        .bytes()
        .await
        .map_err(|e| SourceError::FetchFailed {
            url: feed_config.url.clone(),
            reason: e.to_string(),
        })?;

    let feed = feed_rs::parser::parse(&bytes[..])
        .map_err(|e| SourceError::ParseFailed(format!("{}: {e}", feed_config.url)))?;

    let feed_name = feed_config
        .name
        .as_deref()
        .unwrap_or(feed_config.url.as_str());

    let mut entries = Vec::new();

    let feed_entries = if let Some(limit) = feed_config.limit {
        feed.entries.into_iter().take(limit).collect::<Vec<_>>()
    } else {
        feed.entries
    };

    for entry in feed_entries {
        let title = entry.title.map_or_else(
            || entry.id.clone(),
            |t| t.content,
        );

        // Extract GitHub URL: prefer rel="related" links pointing to github.com,
        // then fall back to any link pointing to github.com.
        let github_url = extract_github_url(&entry.links);

        let Some(repo_url) = github_url else {
            continue;
        };

        let description = entry
            .summary
            .map(|s| s.content)
            .or_else(|| entry.content.and_then(|c| c.body));

        let published = entry.published.or(entry.updated);

        entries.push(FeedEntry {
            title,
            repo_url,
            description,
            published,
            source_name: feed_name.to_string(),
        });
    }

    Ok(entries)
}

/// Extract a GitHub repository URL from a list of feed entry links.
///
/// Priority:
/// 1. `rel="related"` links pointing to `github.com`
/// 2. Any link pointing to `github.com`
fn extract_github_url(links: &[feed_rs::model::Link]) -> Option<Url> {
    // First pass: look for rel="related" links to github.com
    for link in links {
        if link.rel.as_deref() == Some("related")
            && let Ok(url) = Url::parse(&link.href)
            && is_github_repo_url(&url)
        {
            return Some(url);
        }
    }

    // Second pass: any link to github.com
    for link in links {
        if let Ok(url) = Url::parse(&link.href)
            && is_github_repo_url(&url)
        {
            return Some(url);
        }
    }

    None
}

/// Check if a URL points to a GitHub repository (github.com/owner/repo).
fn is_github_repo_url(url: &Url) -> bool {
    url.host_str() == Some("github.com")
        && url
            .path_segments()
            .is_some_and(|mut s| s.next().is_some() && s.next().is_some())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn sample_atom_feed(base_url: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title>AI/ML GitHub Repos Feed</title>
  <link href="{base_url}/feed.xml" rel="self" type="application/atom+xml"/>
  <updated>2026-03-31T00:00:00Z</updated>
  <id>{base_url}/feed.xml</id>

  <entry>
    <id>https://github.com/owner1/repo-alpha</id>
    <title>repo-alpha</title>
    <link href="{base_url}/repo-alpha" rel="alternate" type="text/html"/>
    <link href="https://github.com/owner1/repo-alpha" rel="related" type="text/html"/>
    <summary>A great alpha tool</summary>
    <published>2026-03-30T12:00:00Z</published>
    <updated>2026-03-30T12:00:00Z</updated>
  </entry>

  <entry>
    <id>https://github.com/owner2/repo-beta</id>
    <title>repo-beta</title>
    <link href="{base_url}/repo-beta" rel="alternate" type="text/html"/>
    <link href="https://github.com/owner2/repo-beta" rel="related" type="text/html"/>
    <content type="html">Beta does amazing things</content>
    <updated>2026-03-29T10:00:00Z</updated>
  </entry>

  <entry>
    <id>no-github-link</id>
    <title>entry-without-github</title>
    <link href="{base_url}/no-github" rel="alternate" type="text/html"/>
    <summary>This entry has no GitHub link</summary>
    <updated>2026-03-28T08:00:00Z</updated>
  </entry>

  <entry>
    <id>https://github.com/owner3/repo-gamma</id>
    <title>repo-gamma</title>
    <link href="https://github.com/owner3/repo-gamma" rel="related" type="text/html"/>
    <summary>Gamma project</summary>
    <published>2026-03-27T06:00:00Z</published>
    <updated>2026-03-27T06:00:00Z</updated>
  </entry>
</feed>"#
        )
    }

    #[tokio::test]
    async fn fetches_feed_and_returns_correct_entries() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/feed.xml"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(sample_atom_feed(&server.uri())),
            )
            .mount(&server)
            .await;

        let feeds = vec![FeedConfig {
            url: format!("{}/feed.xml", server.uri()),
            name: Some("test-feed".into()),
            limit: None,
        }];
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .unwrap();

        let source = RssSource::new(feeds, client);
        let entries = source.fetch().await.unwrap();

        // 3 entries have GitHub URLs, 1 does not
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].title, "repo-alpha");
        assert_eq!(
            entries[0].repo_url.as_str(),
            "https://github.com/owner1/repo-alpha"
        );
        assert_eq!(entries[0].source_name, "test-feed");
        assert_eq!(entries[0].description.as_deref(), Some("A great alpha tool"));
        assert!(entries[0].published.is_some());

        assert_eq!(entries[1].title, "repo-beta");
        assert_eq!(
            entries[1].repo_url.as_str(),
            "https://github.com/owner2/repo-beta"
        );
        // Content from <content> tag
        assert_eq!(
            entries[1].description.as_deref(),
            Some("Beta does amazing things")
        );

        assert_eq!(entries[2].title, "repo-gamma");
    }

    #[tokio::test]
    async fn handles_feed_fetch_failure_gracefully() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/good.xml"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(sample_atom_feed(&server.uri())),
            )
            .mount(&server)
            .await;

        // No mock for /bad.xml — will return 404

        let feeds = vec![
            FeedConfig {
                url: format!("{}/bad.xml", server.uri()),
                name: Some("bad-feed".into()),
                limit: None,
            },
            FeedConfig {
                url: format!("{}/good.xml", server.uri()),
                name: Some("good-feed".into()),
                limit: None,
            },
        ];

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .unwrap();

        let source = RssSource::new(feeds, client);
        let entries = source.fetch().await.unwrap();

        // Bad feed parsing fails (404 returns HTML, not valid feed), but good feed works
        assert_eq!(entries.len(), 3);
        assert!(entries.iter().all(|e| e.source_name == "good-feed"));
    }

    #[tokio::test]
    async fn extracts_github_url_from_related_link() {
        let links = vec![
            feed_rs::model::Link {
                href: "https://example.com/page".into(),
                rel: Some("alternate".into()),
                media_type: Some("text/html".into()),
                href_lang: None,
                title: None,
                length: None,
            },
            feed_rs::model::Link {
                href: "https://github.com/owner/repo".into(),
                rel: Some("related".into()),
                media_type: Some("text/html".into()),
                href_lang: None,
                title: None,
                length: None,
            },
        ];

        let result = extract_github_url(&links);
        assert_eq!(result.unwrap().as_str(), "https://github.com/owner/repo");
    }

    #[tokio::test]
    async fn handles_entry_without_github_url() {
        let links = vec![feed_rs::model::Link {
            href: "https://example.com/page".into(),
            rel: Some("alternate".into()),
            media_type: None,
            href_lang: None,
            title: None,
            length: None,
        }];

        let result = extract_github_url(&links);
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn falls_back_to_any_github_link_without_related() {
        let links = vec![feed_rs::model::Link {
            href: "https://github.com/fallback/repo".into(),
            rel: Some("alternate".into()),
            media_type: None,
            href_lang: None,
            title: None,
            length: None,
        }];

        let result = extract_github_url(&links);
        assert_eq!(
            result.unwrap().as_str(),
            "https://github.com/fallback/repo"
        );
    }
}
