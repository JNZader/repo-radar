#![allow(clippy::manual_async_fn)] // Trait uses RPITIT pattern, impls must match

pub mod keyword;

use crate::domain::categorizer::Categorizer;
use crate::domain::model::RepoCandidate;
use crate::infra::error::CategorizerError;

pub use self::keyword::KeywordCategorizer;

/// A no-op categorizer that leaves all candidates with their default category.
#[derive(Debug, Clone)]
pub struct NoopCategorizer;

impl Categorizer for NoopCategorizer {
    fn categorize(
        &self,
        candidates: Vec<RepoCandidate>,
    ) -> Result<Vec<RepoCandidate>, CategorizerError> {
        Ok(candidates)
    }
}

/// Enum dispatch wrapper for all categorizer implementations.
pub enum CategorizerAdapter {
    Noop(NoopCategorizer),
    Keyword(KeywordCategorizer),
}

impl Categorizer for CategorizerAdapter {
    fn categorize(
        &self,
        candidates: Vec<RepoCandidate>,
    ) -> Result<Vec<RepoCandidate>, CategorizerError> {
        match self {
            Self::Noop(c) => c.categorize(candidates),
            Self::Keyword(c) => c.categorize(candidates),
        }
    }
}
