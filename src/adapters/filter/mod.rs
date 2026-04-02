#![allow(clippy::manual_async_fn)] // Trait uses RPITIT pattern, impls must match

pub mod github_metadata;

use std::future::Future;

use crate::domain::filter::Filter;
use crate::domain::model::{FeedEntry, RepoCandidate};
use crate::infra::error::FilterError;

pub use self::github_metadata::GitHubMetadataFilter;

/// A no-op filter that returns an empty candidate list.
#[derive(Debug, Clone)]
pub struct NoopFilter;

impl Filter for NoopFilter {
    fn filter(
        &self,
        _entries: Vec<FeedEntry>,
    ) -> impl Future<Output = Result<Vec<RepoCandidate>, FilterError>> + Send {
        async { Ok(Vec::new()) }
    }
}

/// Enum dispatch wrapper for all filter implementations.
pub enum FilterAdapter {
    Noop(NoopFilter),
    GitHubMetadata(Box<GitHubMetadataFilter>),
}

impl Filter for FilterAdapter {
    fn filter(
        &self,
        entries: Vec<FeedEntry>,
    ) -> impl Future<Output = Result<Vec<RepoCandidate>, FilterError>> + Send {
        async move {
            match self {
                Self::Noop(f) => f.filter(entries).await,
                Self::GitHubMetadata(f) => f.filter(entries).await,
            }
        }
    }
}
