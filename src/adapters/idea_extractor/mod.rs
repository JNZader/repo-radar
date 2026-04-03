pub mod keyword;

pub use self::keyword::KeywordIdeaExtractor;

use crate::domain::idea_extractor::IdeaExtractor;
use crate::domain::model::{CrossRefResult, IdeaReport};
use crate::infra::error::IdeaError;

/// Enum dispatch wrapper for idea extractor implementations.
pub enum IdeaExtractorAdapter {
    Keyword(KeywordIdeaExtractor),
}

impl IdeaExtractor for IdeaExtractorAdapter {
    fn extract(&self, results: &[CrossRefResult]) -> Result<IdeaReport, IdeaError> {
        match self {
            Self::Keyword(x) => x.extract(results),
        }
    }
}
