use std::future::Future;

use super::model::CrossRefResult;
use crate::infra::error::ReporterError;

pub trait Reporter: Send + Sync {
    fn report(&self, results: &[CrossRefResult]) -> impl Future<Output = Result<(), ReporterError>> + Send;
}
