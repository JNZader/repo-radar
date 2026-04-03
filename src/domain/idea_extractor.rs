use super::model::{CrossRefResult, IdeaReport};
use crate::infra::error::IdeaError;

pub trait IdeaExtractor: Send + Sync {
    /// Extract actionable ideas from cross-reference results.
    fn extract(&self, results: &[CrossRefResult]) -> Result<IdeaReport, IdeaError>;
}
