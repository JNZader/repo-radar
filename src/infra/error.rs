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
pub enum CategorizerError {
    #[error("categorization failed: {0}")]
    Failed(String),
}

#[derive(Debug, Error)]
pub enum AnalyzerError {
    #[error("repoforge failed for {repo}: {reason}")]
    RepoforgeError { repo: String, reason: String },
    #[error("repoforge timed out for {repo}")]
    Timeout { repo: String },
    #[error("LLM API error: {0}")]
    LlmError(String),
    #[error("failed to parse analysis: {0}")]
    ParseFailed(String),
}

#[derive(Debug, Error)]
pub enum CrossRefError {
    #[error("failed to load project index: {0}")]
    IndexLoadFailed(String),
    #[error("analysis failed: {0}")]
    AnalysisFailed(String),
    #[error("network error: {0}")]
    Network(String),
}

#[derive(Debug, Error)]
pub enum ReporterError {
    #[error("failed to write report: {0}")]
    WriteFailed(#[from] std::io::Error),
    #[error("template error: {0}")]
    TemplateFailed(String),
    #[error("serialization failed: {0}")]
    SerializationFailed(String),
}

#[derive(Debug, Error)]
pub enum IdeaError {
    #[error("failed to extract ideas: {0}")]
    ExtractionFailed(String),
    #[error("failed to read scan results: {0}")]
    ReadFailed(String),
    #[error("serialization failed: {0}")]
    IdeaSerializationFailed(String),
    #[error("failed to write ideas: {0}")]
    WriteFailed(#[from] std::io::Error),
}

#[derive(Debug, Error)]
pub enum KbError {
    #[error("SQLite error: {0}")]
    Sqlite(String),
    #[error("LLM request failed for {repo}: {reason}")]
    LlmRequest { repo: String, reason: String },
    #[error("JSON parse failed for {repo}: {reason}")]
    ParseFailed { repo: String, reason: String },
    #[error("repoforge export failed for {repo}: {reason}")]
    RepoforgeExport { repo: String, reason: String },
}

#[derive(Debug, Error)]
pub enum PipelineError {
    #[error("source stage failed: {0}")]
    Source(#[from] SourceError),
    #[error("filter stage failed: {0}")]
    Filter(#[from] FilterError),
    #[error("categorizer stage failed: {0}")]
    Categorizer(#[from] CategorizerError),
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
    #[error("cache error: {0}")]
    Cache(String),
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
    fn analyzer_error_parse_failed_display() {
        let err = AnalyzerError::ParseFailed("invalid JSON".into());
        assert_eq!(err.to_string(), "failed to parse analysis: invalid JSON");
    }

    #[test]
    fn crossref_error_network_display() {
        let err = CrossRefError::Network("connection refused".into());
        assert_eq!(err.to_string(), "network error: connection refused");
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
    fn reporter_error_serialization_display() {
        let err = ReporterError::SerializationFailed("invalid UTF-8".into());
        assert_eq!(err.to_string(), "serialization failed: invalid UTF-8");
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

    #[tokio::test]
    async fn source_error_network_from_reqwest() {
        let reqwest_err = reqwest::get("http://[::1]:1")
            .await
            .expect_err("should fail with connection error");
        let source_err = SourceError::from(reqwest_err);
        let display = source_err.to_string();
        assert!(display.contains("network error"));
        assert!(!display.is_empty());
    }

    #[tokio::test]
    async fn filter_error_network_from_reqwest() {
        let reqwest_err = reqwest::get("http://[::1]:1")
            .await
            .expect_err("should fail with connection error");
        let filter_err = FilterError::from(reqwest_err);
        let display = filter_err.to_string();
        assert!(display.contains("network error"));
        assert!(!display.is_empty());
    }

    #[test]
    fn reporter_error_write_failed_from_io() {
        let kinds = [
            std::io::ErrorKind::NotFound,
            std::io::ErrorKind::PermissionDenied,
        ];
        for kind in kinds {
            let io_err = std::io::Error::new(kind, format!("{kind:?} error"));
            let reporter_err = ReporterError::from(io_err);
            let display = reporter_err.to_string();
            assert!(display.contains("failed to write report"), "kind={kind:?}");
            assert!(!display.is_empty());
        }
    }

    #[test]
    fn pipeline_error_display_for_all_variants() {
        let variants: Vec<PipelineError> = vec![
            PipelineError::Source(SourceError::ParseFailed("p".into())),
            PipelineError::Filter(FilterError::GitHubApi("g".into())),
            PipelineError::Categorizer(CategorizerError::Failed("cat".into())),
            PipelineError::Analyzer(AnalyzerError::LlmError("l".into())),
            PipelineError::CrossRef(CrossRefError::AnalysisFailed("c".into())),
            PipelineError::Reporter(ReporterError::TemplateFailed("r".into())),
            PipelineError::Config("cfg".into()),
            PipelineError::SeenStore("ss".into()),
            PipelineError::Cache("cache".into()),
        ];

        let displays: Vec<String> = variants.iter().map(|v| v.to_string()).collect();

        // All non-empty
        for d in &displays {
            assert!(!d.is_empty(), "display should not be empty: {d}");
        }

        // All distinct
        let unique: std::collections::HashSet<&String> = displays.iter().collect();
        assert_eq!(unique.len(), displays.len(), "all display strings should be distinct");
    }

    #[test]
    fn error_types_are_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}

        assert_send_sync::<SourceError>();
        assert_send_sync::<FilterError>();
        assert_send_sync::<CategorizerError>();
        assert_send_sync::<AnalyzerError>();
        assert_send_sync::<CrossRefError>();
        assert_send_sync::<ReporterError>();
        assert_send_sync::<PipelineError>();
    }
}
