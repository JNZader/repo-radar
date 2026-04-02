use std::future::Future;

use super::model::{AnalysisResult, CrossRefResult};
use crate::infra::error::CrossRefError;

pub trait CrossRef: Send + Sync {
    fn cross_reference(&self, results: Vec<AnalysisResult>) -> impl Future<Output = Result<Vec<CrossRefResult>, CrossRefError>> + Send;
}
