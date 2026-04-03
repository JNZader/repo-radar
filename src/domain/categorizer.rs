use super::model::RepoCandidate;
use crate::infra::error::CategorizerError;

pub trait Categorizer: Send + Sync {
    /// Categorize a batch of candidates, setting the `category` field on each.
    fn categorize(
        &self,
        candidates: Vec<RepoCandidate>,
    ) -> Result<Vec<RepoCandidate>, CategorizerError>;
}
