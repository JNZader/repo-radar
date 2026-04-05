use std::path::PathBuf;

use crate::domain::model::CrossRefResult;
use crate::domain::reporter::Reporter;
use crate::infra::error::ReporterError;

/// Writes cross-reference results to a JSON file on disk.
#[derive(Debug, Clone)]
pub struct JsonReporter {
    output_dir: PathBuf,
}

impl JsonReporter {
    pub fn new(output_dir: PathBuf) -> Self {
        Self { output_dir }
    }
}

impl Reporter for JsonReporter {
    fn report(
        &self,
        results: &[CrossRefResult],
    ) -> impl std::future::Future<Output = Result<(), ReporterError>> + Send {
        let json = serde_json::to_string_pretty(results)
            .map_err(|e| ReporterError::SerializationFailed(e.to_string()));
        let path = self
            .output_dir
            .join(format!("report-{}.json", chrono::Utc::now().timestamp()));

        async move {
            let json = json?;
            let parent = path.parent().ok_or_else(|| {
                ReporterError::WriteFailed(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("report path '{}' has no parent directory", path.display()),
                ))
            })?;
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(ReporterError::WriteFailed)?;
            tokio::fs::write(&path, json)
                .await
                .map_err(ReporterError::WriteFailed)?;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::model::{
        AnalysisResult, CrossRefResult, FeedEntry, RepoCandidate, RepoMatch,
    };
    use chrono::Utc;
    use url::Url;

    fn sample_crossref_result(title: &str) -> CrossRefResult {
        CrossRefResult {
            analysis: AnalysisResult {
                candidate: RepoCandidate {
                    entry: FeedEntry {
                        title: title.into(),
                        repo_url: Url::parse("https://github.com/owner/repo").unwrap(),
                        description: Some("desc".into()),
                        published: Some(Utc::now()),
                        source_name: "GitHub Trending".into(),
                    },
                    stars: 500,
                    language: Some("Rust".into()),
                    topics: vec!["cli".into()],
                    fork: false,
                    archived: false,
                    owner: "owner".into(),
                    repo_name: "repo".into(),
                    category: Default::default(),
                    semantic_score: 0.0,
                    pushed_at: None,
                },
                summary: format!("Summary for {title}"),
                key_features: vec!["fast".into()],
                tech_stack: vec!["Rust".into()],
                relevance_score: 0.8,
            },
            matched_repos: vec![RepoMatch {
                own_repo: "my-project".into(),
                relevance: 0.7,
                reason: "Similar stack".into(),
            }],
            ideas: vec!["Use this pattern".into()],
            overall_relevance: 0.75,
        }
    }

    #[tokio::test]
    async fn report_serializes_results_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let reporter = JsonReporter::new(dir.path().to_path_buf());

        let results = vec![
            sample_crossref_result("tool-a"),
            sample_crossref_result("tool-b"),
        ];

        reporter.report(&results).await.unwrap();

        // Find the generated report file
        let mut entries = tokio::fs::read_dir(dir.path()).await.unwrap();
        let entry = entries.next_entry().await.unwrap().expect("report file must exist");
        let path = entry.path();
        assert!(path.file_name().unwrap().to_str().unwrap().starts_with("report-"));
        assert!(path.extension().unwrap() == "json");

        let contents = tokio::fs::read_to_string(&path).await.unwrap();
        let deserialized: Vec<CrossRefResult> = serde_json::from_str(&contents).unwrap();

        assert_eq!(deserialized.len(), 2);
        assert_eq!(deserialized[0].analysis.candidate.entry.title, "tool-a");
        assert_eq!(deserialized[1].analysis.candidate.entry.title, "tool-b");
        assert_eq!(deserialized[0], results[0]);
        assert_eq!(deserialized[1], results[1]);
    }

    #[test]
    fn json_output_snapshot() {
        use chrono::TimeZone;

        let result = CrossRefResult {
            analysis: AnalysisResult {
                candidate: RepoCandidate {
                    entry: FeedEntry {
                        title: "snapshot-tool".into(),
                        repo_url: Url::parse("https://github.com/snapshot/tool").unwrap(),
                        description: Some("A tool for snapshot testing".into()),
                        published: Some(Utc.with_ymd_and_hms(2025, 1, 15, 12, 0, 0).unwrap()),
                        source_name: "GitHub Trending".into(),
                    },
                    stars: 1500,
                    language: Some("Rust".into()),
                    topics: vec!["testing".into(), "snapshot".into()],
                    fork: false,
                    archived: false,
                    owner: "snapshot".into(),
                    repo_name: "tool".into(),
                    category: Default::default(),
                    semantic_score: 0.0,
                    pushed_at: None,
                },
                summary: "A powerful snapshot testing tool for Rust projects".into(),
                key_features: vec!["inline snapshots".into(), "redactions".into()],
                tech_stack: vec!["Rust".into(), "serde".into()],
                relevance_score: 0.92,
            },
            matched_repos: vec![RepoMatch {
                own_repo: "my-test-framework".into(),
                relevance: 0.85,
                reason: "Both focus on testing infrastructure".into(),
            }],
            ideas: vec!["Integrate snapshot testing into CI pipeline".into()],
            overall_relevance: 0.88,
        };

        let json = serde_json::to_string_pretty(&[&result]).unwrap();
        insta::assert_snapshot!(json);
    }

    #[tokio::test]
    async fn report_empty_results_produces_empty_array() {
        let dir = tempfile::tempdir().unwrap();
        let reporter = JsonReporter::new(dir.path().to_path_buf());

        reporter.report(&[]).await.unwrap();

        let mut entries = tokio::fs::read_dir(dir.path()).await.unwrap();
        let entry = entries.next_entry().await.unwrap().expect("report file must exist");
        let contents = tokio::fs::read_to_string(entry.path()).await.unwrap();

        assert_eq!(contents, "[]");
    }
}
