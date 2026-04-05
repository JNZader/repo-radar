#![allow(clippy::manual_async_fn)] // Trait uses RPITIT pattern, impls must match

pub mod github_crossref;

use std::future::Future;

use crate::domain::crossref::CrossRef;
use crate::domain::model::{AnalysisResult, CrossRefResult};
use crate::infra::error::CrossRefError;

use self::github_crossref::GitHubCrossRef;

/// A no-op cross-referencer that passes analysis results through with no repo matches.
/// Used when no GitHub username is configured.
#[derive(Debug, Clone)]
pub struct NoopCrossRef;

impl CrossRef for NoopCrossRef {
    fn cross_reference(&self, results: Vec<AnalysisResult>) -> impl Future<Output = Result<Vec<CrossRefResult>, CrossRefError>> + Send {
        async {
            Ok(results
                .into_iter()
                .map(|analysis| CrossRefResult {
                    analysis,
                    matched_repos: Vec::new(),
                    ideas: Vec::new(),
                    overall_relevance: 0.0,
                })
                .collect())
        }
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
