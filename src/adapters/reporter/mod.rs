#![allow(clippy::manual_async_fn)] // Trait uses RPITIT pattern, impls must match

pub mod console;
pub mod json;
pub mod markdown;

use std::future::Future;

use crate::domain::model::CrossRefResult;
use crate::domain::reporter::Reporter;
use crate::infra::error::ReporterError;

pub use self::console::ConsoleReporter;
pub use self::json::JsonReporter;
pub use self::markdown::MarkdownReporter;

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
    Markdown(MarkdownReporter),
    Json(JsonReporter),
    Console(ConsoleReporter),
}

impl Reporter for ReporterAdapter {
    fn report(&self, results: &[CrossRefResult]) -> impl Future<Output = Result<(), ReporterError>> + Send {
        async move {
            match self {
                Self::Noop(r) => r.report(results).await,
                Self::Markdown(r) => r.report(results).await,
                Self::Json(r) => r.report(results).await,
                Self::Console(r) => r.report(results).await,
            }
        }
    }
}
