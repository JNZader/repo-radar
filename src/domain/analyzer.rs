use std::future::Future;

use super::model::{AnalysisResult, RepoCandidate};
use crate::infra::error::AnalyzerError;

pub trait Analyzer: Send + Sync {
    fn analyze(&self, candidates: Vec<RepoCandidate>) -> impl Future<Output = Result<Vec<AnalysisResult>, AnalyzerError>> + Send;
}
