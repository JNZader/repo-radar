use thiserror::Error;

#[derive(Debug, Error)]
pub enum SourceError {
    #[error("failed to fetch feed from {url}: {reason}")]
    FetchFailed { url: String, reason: String },
    #[error("failed to parse feed: {0}")]
    ParseFailed(String),
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),
}

#[derive(Debug, Error)]
pub enum FilterError {
    #[error("GitHub API error: {0}")]
    GitHubApi(String),
    #[error("rate limit exceeded, resets at {reset_at}")]
    RateLimited { reset_at: String },
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),
}

#[derive(Debug, Error)]
pub enum AnalyzerError {
    #[error("repoforge failed for {repo}: {reason}")]
    RepoforgeError { repo: String, reason: String },
    #[error("repoforge timed out for {repo}")]
    Timeout { repo: String },
    #[error("LLM API error: {0}")]
    LlmError(String),
}

#[derive(Debug, Error)]
pub enum CrossRefError {
    #[error("failed to load project index: {0}")]
    IndexLoadFailed(String),
    #[error("analysis failed: {0}")]
    AnalysisFailed(String),
}

#[derive(Debug, Error)]
pub enum ReporterError {
    #[error("failed to write report: {0}")]
    WriteFailed(#[from] std::io::Error),
    #[error("template error: {0}")]
    TemplateFailed(String),
}

#[derive(Debug, Error)]
pub enum PipelineError {
    #[error("source stage failed: {0}")]
    Source(#[from] SourceError),
    #[error("filter stage failed: {0}")]
    Filter(#[from] FilterError),
    #[error("analyzer stage failed: {0}")]
    Analyzer(#[from] AnalyzerError),
    #[error("cross-reference stage failed: {0}")]
    CrossRef(#[from] CrossRefError),
    #[error("reporter stage failed: {0}")]
    Reporter(#[from] ReporterError),
    #[error("config error: {0}")]
    Config(String),
    #[error("seen store error: {0}")]
    SeenStore(String),
}
