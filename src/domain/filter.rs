use std::future::Future;

use super::model::{FeedEntry, RepoCandidate};
use crate::infra::error::FilterError;

pub trait Filter: Send + Sync {
    fn filter(&self, entries: Vec<FeedEntry>) -> impl Future<Output = Result<Vec<RepoCandidate>, FilterError>> + Send;
}
