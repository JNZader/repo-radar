#![allow(clippy::manual_async_fn)] // Trait uses RPITIT pattern, impls must match

use std::future::Future;

use crate::domain::model::FeedEntry;
use crate::domain::source::Source;
use crate::infra::error::SourceError;

/// A no-op source that returns an empty list. Used for pipeline wiring tests.
#[derive(Debug, Clone)]
pub struct NoopSource;

impl Source for NoopSource {
    fn fetch(&self) -> impl Future<Output = Result<Vec<FeedEntry>, SourceError>> + Send {
        async { Ok(Vec::new()) }
    }

    fn name(&self) -> &'static str {
        "noop"
    }
}

/// Enum dispatch wrapper for all source implementations.
pub enum SourceAdapter {
    Noop(NoopSource),
}

impl Source for SourceAdapter {
    fn fetch(&self) -> impl Future<Output = Result<Vec<FeedEntry>, SourceError>> + Send {
        async {
            match self {
                Self::Noop(s) => s.fetch().await,
            }
        }
    }

    fn name(&self) -> &str {
        match self {
            Self::Noop(s) => s.name(),
        }
    }
}
