use std::future::Future;

use serde::{Deserialize, Serialize};

use crate::domain::model::KbAnalysis;

/// A single actionable idea produced by comparing two repositories.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComparedIdea {
    pub title: String,
    pub description: String,
    /// Effort estimate: `"low"` | `"medium"` | `"high"`
    pub effort: String,
    /// Impact estimate: `"low"` | `"medium"` | `"high"`
    pub impact: String,
}

/// Output of a comparison between a source and a target repository.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompareResult {
    pub source_id: String,
    pub target_id: String,
    pub ideas: Vec<ComparedIdea>,
}

/// Port for comparing two `KbAnalysis` entries and extracting actionable ideas.
pub trait CompareService: Send + Sync {
    fn compare(
        &self,
        source: &KbAnalysis,
        target: &KbAnalysis,
    ) -> impl Future<Output = Result<CompareResult, CompareError>> + Send;
}

/// Errors that can occur during repository comparison.
#[derive(Debug, thiserror::Error)]
pub enum CompareError {
    #[error("LLM request failed: {0}")]
    LlmError(String),
    #[error("parse failed after retry: {raw}")]
    ParseFailed { raw: String },
}
