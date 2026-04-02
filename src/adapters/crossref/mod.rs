#![allow(clippy::manual_async_fn)] // Trait uses RPITIT pattern, impls must match

pub mod github_crossref;

use std::future::Future;

use crate::domain::crossref::CrossRef;
use crate::domain::model::{AnalysisResult, CrossRefResult};
use crate::infra::error::CrossRefError;

use self::github_crossref::GitHubCrossRef;

/// A no-op cross-referencer that returns an empty result list.
#[derive(Debug, Clone)]
pub struct NoopCrossRef;

impl CrossRef for NoopCrossRef {
    fn cross_reference(&self, _results: Vec<AnalysisResult>) -> impl Future<Output = Result<Vec<CrossRefResult>, CrossRefError>> + Send {
        async { Ok(Vec::new()) }
    }
}

/// Enum dispatch wrapper for all cross-reference implementations.
pub enum CrossRefAdapter {
    Noop(NoopCrossRef),
    GitHub(Box<GitHubCrossRef>),
}

impl CrossRef for CrossRefAdapter {
    fn cross_reference(&self, results: Vec<AnalysisResult>) -> impl Future<Output = Result<Vec<CrossRefResult>, CrossRefError>> + Send {
        async move {
            match self {
                Self::Noop(x) => x.cross_reference(results).await,
                Self::GitHub(x) => x.cross_reference(results).await,
            }
        }
    }
}
