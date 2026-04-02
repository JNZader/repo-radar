#![allow(clippy::manual_async_fn)] // Trait uses RPITIT pattern, impls must match

use std::future::Future;
use std::path::PathBuf;
use std::time::Duration;

use serde::Deserialize;
use tracing::warn;

use crate::domain::analyzer::Analyzer;
use crate::domain::model::{AnalysisResult, RepoCandidate};
use crate::infra::error::AnalyzerError;

/// Intermediate struct for deserializing repoforge CLI JSON output.
#[derive(Debug, Deserialize)]
struct RepoforgeOutput {
    summary: String,
    #[serde(default)]
    key_features: Vec<String>,
    #[serde(default)]
    tech_stack: Vec<String>,
    #[serde(default)]
    relevance_score: f64,
}

/// Runs the RepoForge CLI as a subprocess to analyze repositories.
pub struct RepoforgeAnalyzer {
    repoforge_path: PathBuf,
    timeout: Duration,
}

impl RepoforgeAnalyzer {
    /// Create a new `RepoforgeAnalyzer` with the path to the repoforge binary
    /// and a per-candidate timeout in seconds.
    #[must_use]
    pub fn new(repoforge_path: PathBuf, timeout_secs: u64) -> Self {
        Self {
            repoforge_path,
            timeout: Duration::from_secs(timeout_secs),
        }
    }

    /// Analyze a single candidate by invoking the repoforge CLI.
    async fn analyze_one(
        &self,
        candidate: &RepoCandidate,
    ) -> Result<AnalysisResult, AnalyzerError> {
        let repo_url = candidate.entry.repo_url.as_str();

        let future = tokio::process::Command::new(&self.repoforge_path)
            .arg("analyze")
            .arg(repo_url)
            .arg("--json")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output();

        let output = tokio::time::timeout(self.timeout, future)
            .await
            .map_err(|_| AnalyzerError::Timeout {
                repo: repo_url.to_string(),
            })?
            .map_err(|e| AnalyzerError::RepoforgeError {
                repo: repo_url.to_string(),
                reason: e.to_string(),
            })?;

        if !output.status.success() {
            return Err(AnalyzerError::RepoforgeError {
                repo: repo_url.to_string(),
                reason: format!(
                    "exited with code {}",
                    output.status.code().unwrap_or(-1)
                ),
            });
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let parsed: RepoforgeOutput =
            serde_json::from_str(&stdout).map_err(|e| AnalyzerError::ParseFailed(e.to_string()))?;

        Ok(AnalysisResult {
            candidate: candidate.clone(),
            summary: parsed.summary,
            key_features: parsed.key_features,
            tech_stack: parsed.tech_stack,
            relevance_score: parsed.relevance_score,
        })
    }
}

impl Analyzer for RepoforgeAnalyzer {
    fn analyze(
        &self,
        candidates: Vec<RepoCandidate>,
    ) -> impl Future<Output = Result<Vec<AnalysisResult>, AnalyzerError>> + Send {
        async move {
            let mut results = Vec::new();

            for candidate in &candidates {
                match self.analyze_one(candidate).await {
                    Ok(result) => results.push(result),
                    Err(e) => {
                        warn!(
                            repo = %candidate.entry.repo_url,
                            error = %e,
                            "failed to analyze candidate, skipping"
                        );
                    }
                }
            }

            Ok(results)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use url::Url;

    use crate::domain::model::FeedEntry;

    fn sample_candidate(repo_url: &str) -> RepoCandidate {
        RepoCandidate {
            entry: FeedEntry {
                title: "test-repo".into(),
                repo_url: Url::parse(repo_url).unwrap(),
                description: Some("A test repo".into()),
                published: Some(Utc::now()),
                source_name: "test".into(),
            },
            stars: 100,
            language: Some("Rust".into()),
            topics: vec!["cli".into()],
            fork: false,
            archived: false,
            owner: "owner".into(),
            repo_name: "test-repo".into(),
        }
    }

    fn fixture_path(name: &str) -> PathBuf {
        let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        p.push("tests/fixtures");
        p.push(name);
        p
    }

    #[tokio::test]
    async fn happy_path_parses_json_output() {
        let analyzer = RepoforgeAnalyzer::new(fixture_path("fake_repoforge.sh"), 10);
        let candidates = vec![sample_candidate("https://github.com/owner/test-repo")];

        let results = analyzer.analyze(candidates).await.unwrap();

        assert_eq!(results.len(), 1);
        let r = &results[0];
        assert_eq!(r.summary, "A great tool for testing");
        assert_eq!(r.key_features, vec!["fast", "reliable"]);
        assert_eq!(r.tech_stack, vec!["Rust", "tokio"]);
        assert!((r.relevance_score - 0.85).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn timeout_skips_candidate() {
        let analyzer = RepoforgeAnalyzer::new(fixture_path("fake_repoforge_slow.sh"), 1);
        let candidates = vec![sample_candidate("https://github.com/owner/slow-repo")];

        let results = analyzer.analyze(candidates).await.unwrap();

        assert!(results.is_empty(), "timed-out candidate should be skipped");
    }

    #[tokio::test]
    async fn parse_error_skips_candidate() {
        let analyzer = RepoforgeAnalyzer::new(fixture_path("fake_repoforge_bad_json.sh"), 10);
        let candidates = vec![sample_candidate("https://github.com/owner/bad-json-repo")];

        let results = analyzer.analyze(candidates).await.unwrap();

        assert!(results.is_empty(), "malformed JSON candidate should be skipped");
    }

    #[tokio::test]
    async fn subprocess_failure_skips_candidate() {
        let analyzer = RepoforgeAnalyzer::new(fixture_path("fake_repoforge_bad.sh"), 10);
        let candidates = vec![sample_candidate("https://github.com/owner/fail-repo")];

        let results = analyzer.analyze(candidates).await.unwrap();

        assert!(results.is_empty(), "non-zero exit candidate should be skipped");
    }

    #[tokio::test]
    async fn empty_candidates_returns_empty_vec() {
        let analyzer = RepoforgeAnalyzer::new(fixture_path("fake_repoforge.sh"), 10);
        let candidates = vec![];

        let results = analyzer.analyze(candidates).await.unwrap();

        assert!(results.is_empty());
    }
}
