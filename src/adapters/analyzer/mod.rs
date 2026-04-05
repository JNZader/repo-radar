#![allow(clippy::manual_async_fn)] // Trait uses RPITIT pattern, impls must match

pub mod repoforge;

use std::future::Future;

use crate::domain::analyzer::Analyzer;
use crate::domain::model::{AnalysisResult, RepoCandidate};
use crate::infra::error::AnalyzerError;

pub use self::repoforge::RepoforgeAnalyzer;

/// A no-op analyzer that passes candidates through with empty analysis fields.
/// Used when no real analyzer (e.g. repoforge) is configured.
#[derive(Debug, Clone)]
pub struct NoopAnalyzer;

impl Analyzer for NoopAnalyzer {
    fn analyze(&self, candidates: Vec<RepoCandidate>) -> impl Future<Output = Result<Vec<AnalysisResult>, AnalyzerError>> + Send {
        async {
            Ok(candidates
                .into_iter()
                .map(|candidate| AnalysisResult {
                    candidate,
                    summary: String::new(),
                    key_features: Vec::new(),
                    tech_stack: Vec::new(),
                    relevance_score: 0.0,
                })
                .collect())
        }
    }
}

/// Enum dispatch wrapper for all analyzer implementations.
pub enum AnalyzerAdapter {
    Noop(NoopAnalyzer),
    Repoforge(RepoforgeAnalyzer),
}

impl Analyzer for AnalyzerAdapter {
    fn analyze(&self, candidates: Vec<RepoCandidate>) -> impl Future<Output = Result<Vec<AnalysisResult>, AnalyzerError>> + Send {
        async move {
            match self {
                Self::Noop(a) => a.analyze(candidates).await,
                Self::Repoforge(a) => a.analyze(candidates).await,
            }
        }
    }
}
