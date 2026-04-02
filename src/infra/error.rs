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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_error_fetch_failed_display() {
        let err = SourceError::FetchFailed {
            url: "https://example.com".into(),
            reason: "timeout".into(),
        };
        assert_eq!(
            err.to_string(),
            "failed to fetch feed from https://example.com: timeout"
        );
    }

    #[test]
    fn source_error_parse_failed_display() {
        let err = SourceError::ParseFailed("bad XML".into());
        assert_eq!(err.to_string(), "failed to parse feed: bad XML");
    }

    #[test]
    fn filter_error_github_api_display() {
        let err = FilterError::GitHubApi("not found".into());
        assert_eq!(err.to_string(), "GitHub API error: not found");
    }

    #[test]
    fn filter_error_rate_limited_display() {
        let err = FilterError::RateLimited {
            reset_at: "2026-01-01T00:00:00Z".into(),
        };
        assert_eq!(
            err.to_string(),
            "rate limit exceeded, resets at 2026-01-01T00:00:00Z"
        );
    }

    #[test]
    fn analyzer_error_repoforge_display() {
        let err = AnalyzerError::RepoforgeError {
            repo: "owner/repo".into(),
            reason: "crash".into(),
        };
        assert_eq!(
            err.to_string(),
            "repoforge failed for owner/repo: crash"
        );
    }

    #[test]
    fn analyzer_error_timeout_display() {
        let err = AnalyzerError::Timeout {
            repo: "owner/repo".into(),
        };
        assert_eq!(err.to_string(), "repoforge timed out for owner/repo");
    }

    #[test]
    fn analyzer_error_llm_display() {
        let err = AnalyzerError::LlmError("rate limited".into());
        assert_eq!(err.to_string(), "LLM API error: rate limited");
    }

    #[test]
    fn crossref_error_display() {
        let err = CrossRefError::IndexLoadFailed("not found".into());
        assert_eq!(err.to_string(), "failed to load project index: not found");

        let err = CrossRefError::AnalysisFailed("corrupt data".into());
        assert_eq!(err.to_string(), "analysis failed: corrupt data");
    }

    #[test]
    fn reporter_error_template_display() {
        let err = ReporterError::TemplateFailed("missing var".into());
        assert_eq!(err.to_string(), "template error: missing var");
    }

    #[test]
    fn reporter_error_from_io() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file missing");
        let reporter_err = ReporterError::from(io_err);
        assert!(reporter_err.to_string().contains("failed to write report"));
    }

    #[test]
    fn pipeline_error_from_source() {
        let source_err = SourceError::ParseFailed("bad".into());
        let pipeline_err = PipelineError::from(source_err);
        assert!(pipeline_err.to_string().contains("source stage failed"));
    }

    #[test]
    fn pipeline_error_from_filter() {
        let filter_err = FilterError::GitHubApi("oops".into());
        let pipeline_err = PipelineError::from(filter_err);
        assert!(pipeline_err.to_string().contains("filter stage failed"));
    }

    #[test]
    fn pipeline_error_from_analyzer() {
        let err = AnalyzerError::LlmError("fail".into());
        let pipeline_err = PipelineError::from(err);
        assert!(pipeline_err.to_string().contains("analyzer stage failed"));
    }

    #[test]
    fn pipeline_error_from_crossref() {
        let err = CrossRefError::AnalysisFailed("fail".into());
        let pipeline_err = PipelineError::from(err);
        assert!(
            pipeline_err
                .to_string()
                .contains("cross-reference stage failed")
        );
    }

    #[test]
    fn pipeline_error_from_reporter() {
        let io_err = std::io::Error::other("disk full");
        let reporter_err = ReporterError::from(io_err);
        let pipeline_err = PipelineError::from(reporter_err);
        assert!(pipeline_err.to_string().contains("reporter stage failed"));
    }

    #[test]
    fn pipeline_error_config_display() {
        let err = PipelineError::Config("bad toml".into());
        assert_eq!(err.to_string(), "config error: bad toml");
    }

    #[test]
    fn pipeline_error_seen_store_display() {
        let err = PipelineError::SeenStore("corrupt".into());
        assert_eq!(err.to_string(), "seen store error: corrupt");
    }
}
