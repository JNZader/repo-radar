use std::future::Future;

use super::model::FeedEntry;
use crate::infra::error::SourceError;

pub trait Source: Send + Sync {
    fn fetch(&self) -> impl Future<Output = Result<Vec<FeedEntry>, SourceError>> + Send;
    fn name(&self) -> &str;
}
