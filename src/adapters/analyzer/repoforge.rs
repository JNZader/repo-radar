#![allow(clippy::manual_async_fn)] // Trait uses RPITIT pattern, impls must match

use std::future::Future;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use tracing::warn;

use crate::domain::analyzer::Analyzer;
use crate::domain::model::{AnalysisResult, RepoCandidate};
use crate::infra::error::AnalyzerError;
use crate::infra::repoforge::RepoforgeRunner;

/// Runs the RepoForge CLI as a subprocess to analyze repositories.
///
/// Workflow per candidate:
/// 1. `git clone --depth 1 --single-branch <url> <tmpdir>`
/// 2. `repoforge export -w <tmpdir> --no-contents -q` (via `RepoforgeRunner`)
/// 3. Parse the markdown output for tech_stack and key definitions.
pub struct RepoforgeAnalyzer {
    runner: RepoforgeRunner,
    git_path: PathBuf,
}

impl RepoforgeAnalyzer {
    /// Create a new `RepoforgeAnalyzer` with the path to the repoforge binary
    /// and a per-candidate timeout in seconds.
    #[must_use]
    pub fn new(repoforge_path: PathBuf, timeout_secs: u64) -> Self {
        Self {
            runner: RepoforgeRunner::new(
                repoforge_path,
                Duration::from_secs(timeout_secs),
            ),
            git_path: PathBuf::from("git"),
        }
    }

    /// Create a `RepoforgeAnalyzer` from an existing `RepoforgeRunner`.
    #[must_use]
    pub fn with_runner(runner: RepoforgeRunner) -> Self {
        Self {
            runner,
            git_path: PathBuf::from("git"),
        }
    }

    /// Override the git binary path (useful for testing with a fake git script).
    #[must_use]
    pub fn with_git_path(mut self, git_path: PathBuf) -> Self {
        self.git_path = git_path;
        self
    }

    /// Analyze a single candidate: clone → export → parse.
    async fn analyze_one(
        &self,
        candidate: &RepoCandidate,
    ) -> Result<AnalysisResult, AnalyzerError> {
        let repo_url = candidate.entry.repo_url.as_str();

        // Step 1: create a temp dir and shallow-clone the repo into it.
        let tmp_dir = tempfile::TempDir::new().map_err(|e| AnalyzerError::RepoforgeError {
            repo: repo_url.to_string(),
            reason: format!("failed to create temp dir: {e}"),
        })?;

        let clone_future = tokio::process::Command::new(&self.git_path)
            .args(["clone", "--depth", "1", "--single-branch", repo_url])
            .arg(tmp_dir.path())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .output();

        let clone_output = tokio::time::timeout(self.runner.timeout, clone_future)
            .await
            .map_err(|_| AnalyzerError::Timeout {
                repo: repo_url.to_string(),
            })?
            .map_err(|e| AnalyzerError::RepoforgeError {
                repo: repo_url.to_string(),
                reason: e.to_string(),
            })?;

        if !clone_output.status.success() {
            return Err(AnalyzerError::RepoforgeError {
                repo: repo_url.to_string(),
                reason: format!(
                    "git clone failed with code {}",
                    clone_output.status.code().unwrap_or(-1)
                ),
            });
        }

        // Step 2: run `repoforge export --no-contents` on the cloned directory.
        let markdown = self
            .runner
            .export_no_contents(tmp_dir.path())
            .await
            .map_err(|e| AnalyzerError::RepoforgeError {
                repo: repo_url.to_string(),
                reason: e.to_string(),
            })?;
        let markdown = markdown.as_str();
        let tech_stack = parse_tech_stack(&markdown);
        let key_features = parse_key_definitions(&markdown);

        // Use the candidate's description as summary (HTML stripped for readability).
        let summary = candidate
            .entry
            .description
            .as_deref()
            .map(strip_html)
            .unwrap_or_default();

        Ok(AnalysisResult {
            candidate: candidate.clone(),
            summary,
            key_features,
            tech_stack,
            relevance_score: candidate.semantic_score,
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

/// Strip HTML tags, replacing them with spaces so adjacent words don't merge.
fn strip_html(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut in_tag = false;
    for ch in text.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                result.push(' ');
            }
            _ if !in_tag => result.push(ch),
            _ => {}
        }
    }
    // Collapse multiple spaces and trim.
    result
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Parse the tech stack from a `repoforge export` markdown output.
///
/// Looks for a line like `- **Tech stack**: Rust, tokio, serde`.
fn parse_tech_stack(markdown: &str) -> Vec<String> {
    for line in markdown.lines() {
        if let Some(rest) = line.strip_prefix("- **Tech stack**: ") {
            let rest = rest.trim();
            if !rest.is_empty() && rest != "not detected" {
                return rest.split(", ").map(str::trim).map(String::from).collect();
            }
        }
    }
    Vec::new()
}

/// Parse key definitions from the `## Key Definitions` section.
///
/// Extracts names from backtick-quoted identifiers or bold-formatted names.
fn parse_key_definitions(markdown: &str) -> Vec<String> {
    let mut in_section = false;
    let mut results = Vec::new();

    for line in markdown.lines() {
        if line.starts_with("## Key Definitions") {
            in_section = true;
            continue;
        }
        if in_section && line.starts_with("## ") {
            break;
        }
        if !in_section {
            continue;
        }

        let trimmed = line.trim();
        if !trimmed.starts_with("- ") {
            continue;
        }

        let item = trimmed.trim_start_matches("- ");
        // Try backtick: `name`
        if let Some(start) = item.find('`') {
            let after = &item[start + 1..];
            if let Some(end) = after.find('`') {
                let name = &after[..end];
                if !name.is_empty() {
                    results.push(name.to_string());
                    continue;
                }
            }
        }
        // Try bold: **name**
        if let Some(rest) = item.strip_prefix("**") {
            if let Some(end) = rest.find("**") {
                let name = &rest[..end];
                if !name.is_empty() {
                    results.push(name.to_string());
                    continue;
                }
            }
        }
        // Fallback: first word
        if let Some(word) = item.split_whitespace().next() {
            if !word.is_empty() {
                results.push(word.to_string());
            }
        }
    }

    results
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
            category: Default::default(),
            semantic_score: 0.42,
            pushed_at: None,
        }
    }

    fn fixture_path(name: &str) -> PathBuf {
        let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        p.push("tests/fixtures");
        p.push(name);
        p
    }

    fn fake_git() -> PathBuf {
        fixture_path("fake_git.sh")
    }

    fn fake_git_fail() -> PathBuf {
        fixture_path("fake_git_fail.sh")
    }

    fn fake_git_slow() -> PathBuf {
        fixture_path("fake_git_slow.sh")
    }

    #[tokio::test]
    async fn happy_path_parses_markdown_output() {
        let analyzer = RepoforgeAnalyzer::new(fixture_path("fake_repoforge.sh"), 10)
            .with_git_path(fake_git());
        let candidates = vec![sample_candidate("https://github.com/owner/test-repo")];

        let results = analyzer.analyze(candidates).await.unwrap();

        assert_eq!(results.len(), 1);
        let r = &results[0];
        assert_eq!(r.tech_stack, vec!["Rust", "tokio"]);
        assert!(
            r.key_features.contains(&"analyze_one".to_string()),
            "key_features = {:?}",
            r.key_features
        );
        assert_eq!(r.summary, "A test repo");
        assert!((r.relevance_score - 0.42).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn timeout_on_git_clone_skips_candidate() {
        let analyzer = RepoforgeAnalyzer::new(fixture_path("fake_repoforge.sh"), 1)
            .with_git_path(fake_git_slow());
        let candidates = vec![sample_candidate("https://github.com/owner/slow-repo")];

        let results = analyzer.analyze(candidates).await.unwrap();

        assert!(results.is_empty(), "timed-out clone should be skipped");
    }

    #[tokio::test]
    async fn git_clone_failure_skips_candidate() {
        let analyzer = RepoforgeAnalyzer::new(fixture_path("fake_repoforge.sh"), 10)
            .with_git_path(fake_git_fail());
        let candidates = vec![sample_candidate("https://github.com/owner/fail-repo")];

        let results = analyzer.analyze(candidates).await.unwrap();

        assert!(results.is_empty(), "failed clone should be skipped");
    }

    #[tokio::test]
    async fn repoforge_export_failure_skips_candidate() {
        let analyzer = RepoforgeAnalyzer::new(fixture_path("fake_repoforge_bad.sh"), 10)
            .with_git_path(fake_git());
        let candidates = vec![sample_candidate("https://github.com/owner/bad-export-repo")];

        let results = analyzer.analyze(candidates).await.unwrap();

        assert!(results.is_empty(), "failed export should be skipped");
    }

    #[tokio::test]
    async fn empty_candidates_returns_empty_vec() {
        let analyzer = RepoforgeAnalyzer::new(fixture_path("fake_repoforge.sh"), 10)
            .with_git_path(fake_git());

        let results = analyzer.analyze(vec![]).await.unwrap();

        assert!(results.is_empty());
    }

    #[test]
    fn parse_tech_stack_extracts_comma_separated() {
        let md = "# Repo\n\n## Project Overview\n\n- **Tech stack**: Rust, tokio, serde\n";
        assert_eq!(parse_tech_stack(md), vec!["Rust", "tokio", "serde"]);
    }

    #[test]
    fn parse_tech_stack_returns_empty_when_not_detected() {
        let md = "- **Tech stack**: not detected\n";
        assert!(parse_tech_stack(md).is_empty());
    }

    #[test]
    fn parse_key_definitions_extracts_backtick_names() {
        let md = "## Key Definitions\n\n- `analyze_one` — does analysis\n- `RepoforgeAnalyzer` — main struct\n\n## Next Section\n";
        let defs = parse_key_definitions(md);
        assert!(defs.contains(&"analyze_one".to_string()));
        assert!(defs.contains(&"RepoforgeAnalyzer".to_string()));
    }

    #[test]
    fn parse_key_definitions_stops_at_next_heading() {
        let md = "## Key Definitions\n\n- `foo` — foo fn\n\n## Directory Tree\n\n- `bar` — should not appear\n";
        let defs = parse_key_definitions(md);
        assert!(defs.contains(&"foo".to_string()));
        assert!(!defs.contains(&"bar".to_string()));
    }

    #[test]
    fn strip_html_removes_tags_and_collapses_whitespace() {
        let input = "<p><img src=\"x\"/></p><h1>A great CLI tool</h1>";
        assert_eq!(strip_html(input), "A great CLI tool");
    }
}
