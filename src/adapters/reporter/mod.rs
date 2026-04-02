#![allow(clippy::manual_async_fn)] // Trait uses RPITIT pattern, impls must match

use std::future::Future;

use crate::domain::model::CrossRefResult;
use crate::domain::reporter::Reporter;
use crate::infra::error::ReporterError;

/// A no-op reporter that does nothing.
#[derive(Debug, Clone)]
pub struct NoopReporter;

impl Reporter for NoopReporter {
    fn report(&self, _results: &[CrossRefResult]) -> impl Future<Output = Result<(), ReporterError>> + Send {
        async { Ok(()) }
    }
}

/// Enum dispatch wrapper for all reporter implementations.
pub enum ReporterAdapter {
    Noop(NoopReporter),
}

impl Reporter for ReporterAdapter {
    fn report(&self, results: &[CrossRefResult]) -> impl Future<Output = Result<(), ReporterError>> + Send {
        let _ = results;
        async { Ok(()) }
    }
}
