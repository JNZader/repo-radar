#![allow(clippy::manual_async_fn)] // Trait uses RPITIT pattern, impls must match

pub mod github_trending;
pub mod hackernews;
pub mod reddit;
pub mod rss;

use std::future::Future;
use std::pin::Pin;

use tracing::info;

use crate::domain::model::FeedEntry;
use crate::domain::source::Source;
use crate::infra::error::SourceError;

pub use self::github_trending::GitHubTrendingSource;
pub use self::hackernews::HackerNewsSource;
pub use self::reddit::RedditSource;
pub use self::rss::RssSource;

/// A no-op source that returns an empty list. Used for pipeline wiring tests.
#[derive(Debug, Clone)]
pub struct NoopSource;

impl Source for NoopSource {
    fn fetch(&self) -> impl Future<Output = Result<Vec<FeedEntry>, SourceError>> + Send {
        async { Ok(Vec::new()) }
    }

    fn name(&self) -> &'static str {
        "noop"
    }
}

/// Enum dispatch wrapper for all source implementations.
pub enum SourceAdapter {
    Noop(NoopSource),
    Rss(RssSource),
    GitHubTrending(GitHubTrendingSource),
    HackerNews(HackerNewsSource),
    Reddit(RedditSource),
    /// Aggregates multiple sources into one.
    Multi(MultiSource),
}

impl Source for SourceAdapter {
    fn fetch(&self) -> impl Future<Output = Result<Vec<FeedEntry>, SourceError>> + Send {
        // Use Box::pin to break infinite size from Multi→SourceAdapter→Multi recursion.
        let fut: Pin<Box<dyn Future<Output = Result<Vec<FeedEntry>, SourceError>> + Send + '_>> =
            match self {
                Self::Noop(s) => Box::pin(s.fetch()),
                Self::Rss(s) => Box::pin(s.fetch()),
                Self::GitHubTrending(s) => Box::pin(s.fetch()),
                Self::HackerNews(s) => Box::pin(s.fetch()),
                Self::Reddit(s) => Box::pin(s.fetch()),
                Self::Multi(s) => Box::pin(s.fetch()),
            };
        async move { fut.await }
    }

    fn name(&self) -> &str {
        match self {
            Self::Noop(s) => s.name(),
            Self::Rss(s) => s.name(),
            Self::GitHubTrending(s) => s.name(),
            Self::HackerNews(s) => s.name(),
            Self::Reddit(s) => s.name(),
            Self::Multi(s) => s.name(),
        }
    }
}

/// Aggregates multiple `SourceAdapter` instances, merging their results.
pub struct MultiSource {
    sources: Vec<SourceAdapter>,
}

impl MultiSource {
    #[must_use]
    pub fn new(sources: Vec<SourceAdapter>) -> Self {
        Self { sources }
    }
}

impl Source for MultiSource {
    fn fetch(&self) -> impl Future<Output = Result<Vec<FeedEntry>, SourceError>> + Send {
        let sources = &self.sources;
        async move {
            let mut all_entries = Vec::new();
            for source in sources {
                info!(source = source.name(), "fetching from source");
                match source.fetch().await {
                    Ok(entries) => {
                        info!(source = source.name(), count = entries.len(), "entries fetched");
                        all_entries.extend(entries);
                    }
                    Err(e) => {
                        tracing::warn!(source = source.name(), error = %e, "source failed, skipping");
                    }
                }
            }
            Ok(all_entries)
        }
    }

    fn name(&self) -> &'static str {
        "multi"
    }
}
