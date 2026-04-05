#![allow(clippy::manual_async_fn)] // Trait uses RPITIT pattern, impls must match

use std::fmt::Write as _;
use std::future::Future;
use std::path::PathBuf;

use chrono::Utc;

use crate::domain::model::CrossRefResult;
use crate::domain::reporter::Reporter;
use crate::infra::error::ReporterError;

/// Writes cross-reference results to a Markdown report file.
pub struct MarkdownReporter {
    output_dir: PathBuf,
}

impl MarkdownReporter {
    #[must_use]
    pub fn new(output_dir: PathBuf) -> Self {
        Self { output_dir }
    }

    fn format_entry(result: &CrossRefResult) -> String {
        let analysis = &result.analysis;
        let candidate = &analysis.candidate;
        let mut buf = String::new();

        writeln!(
            buf,
            "## {} (relevance: {:.0}%)\n",
            candidate.entry.title,
            result.overall_relevance * 100.0
        )
        .expect("write to String cannot fail");

        writeln!(buf, "**URL:** {}", candidate.entry.repo_url).expect("write to String cannot fail");
        writeln!(buf, "**Stars:** {} | **Language:** {} | **Category:** {}", candidate.stars, candidate.language.as_deref().unwrap_or("N/A"), candidate.category)
            .expect("write to String cannot fail");

        if !candidate.topics.is_empty() {
            writeln!(buf, "**Topics:** {}", candidate.topics.join(", ")).expect("write to String cannot fail");
        }

        writeln!(buf, "\n### Summary\n\n{}", analysis.summary).expect("write to String cannot fail");

        if !analysis.key_features.is_empty() {
            writeln!(buf, "\n### Key Features\n").expect("write to String cannot fail");
            for feature in &analysis.key_features {
                writeln!(buf, "- {feature}").expect("write to String cannot fail");
            }
        }

        if !analysis.tech_stack.is_empty() {
            writeln!(buf, "\n### Tech Stack\n").expect("write to String cannot fail");
            for tech in &analysis.tech_stack {
                writeln!(buf, "- {tech}").expect("write to String cannot fail");
            }
        }

        if !result.matched_repos.is_empty() {
            writeln!(buf, "\n### Matched Repos\n").expect("write to String cannot fail");
            for m in &result.matched_repos {
                writeln!(buf, "- **{}** ({:.0}%): {}", m.own_repo, m.relevance * 100.0, m.reason)
                    .expect("write to String cannot fail");
            }
        }

        if !result.ideas.is_empty() {
            writeln!(buf, "\n### Ideas\n").expect("write to String cannot fail");
            for idea in &result.ideas {
                writeln!(buf, "- {idea}").expect("write to String cannot fail");
            }
        }

        buf
    }
}

impl Reporter for MarkdownReporter {
    fn report(
        &self,
        results: &[CrossRefResult],
    ) -> impl Future<Output = Result<(), ReporterError>> + Send {
        let output_dir = self.output_dir.clone();
        let content = if results.is_empty() {
            "# Repo Radar Report\n\nNo results found.\n".to_string()
        } else {
            let mut sorted: Vec<&CrossRefResult> = results.iter().collect();
            sorted.sort_by(|a, b| b.overall_relevance.partial_cmp(&a.overall_relevance).unwrap_or(std::cmp::Ordering::Equal));

            let mut buf = String::from("# Repo Radar Report\n\n");
            for result in sorted {
                buf.push_str(&Self::format_entry(result));
                buf.push('\n');
            }
            buf
        };

        let timestamp = Utc::now().format("%Y%m%d-%H%M%S");
        let filename = format!("report-{timestamp}.md");
        let path = output_dir.join(filename);

        async move {
            tokio::fs::create_dir_all(&output_dir).await?;
            tokio::fs::write(&path, content).await?;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use url::Url;

    use crate::domain::model::{AnalysisResult, FeedEntry, RepoCandidate, RepoMatch};

    #[test]
    fn markdown_output_snapshot() {
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

        let markdown = MarkdownReporter::format_entry(&result);
        insta::assert_snapshot!(markdown);
    }

    fn sample_crossref_result(title: &str, relevance: f64) -> CrossRefResult {
        CrossRefResult {
            analysis: AnalysisResult {
                candidate: RepoCandidate {
                    entry: FeedEntry {
                        title: title.into(),
                        repo_url: Url::parse(&format!("https://github.com/owner/{title}")).unwrap(),
                        description: Some(format!("Description of {title}")),
                        published: Some(Utc::now()),
                        source_name: "test".into(),
                    },
                    stars: 500,
                    language: Some("Rust".into()),
                    topics: vec!["cli".into()],
                    fork: false,
                    archived: false,
                    owner: "owner".into(),
                    repo_name: title.into(),
                    category: Default::default(),
                    semantic_score: 0.0,
                },
                summary: format!("Summary for {title}"),
                key_features: vec!["fast".into(), "safe".into()],
                tech_stack: vec!["Rust".into(), "tokio".into()],
                relevance_score: relevance,
            },
            matched_repos: vec![RepoMatch {
                own_repo: "my-project".into(),
                relevance: 0.8,
                reason: "Similar stack".into(),
            }],
            ideas: vec!["Try this pattern".into()],
            overall_relevance: relevance,
        }
    }

    #[tokio::test]
    async fn happy_path_three_results_sorted_by_relevance() {
        let dir = tempfile::tempdir().unwrap();
        let reporter = MarkdownReporter::new(dir.path().to_path_buf());

        let results = vec![
            sample_crossref_result("low-tool", 0.3),
            sample_crossref_result("high-tool", 0.95),
            sample_crossref_result("mid-tool", 0.6),
        ];

        reporter.report(&results).await.unwrap();

        let entries: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert_eq!(entries.len(), 1);

        let content = std::fs::read_to_string(entries[0].path()).unwrap();

        // Verify header
        assert!(content.starts_with("# Repo Radar Report"));

        // Verify all three H2 sections are present
        assert!(content.contains("## high-tool"));
        assert!(content.contains("## mid-tool"));
        assert!(content.contains("## low-tool"));

        // Verify sorted by relevance descending: high appears before mid, mid before low
        let pos_high = content.find("## high-tool").unwrap();
        let pos_mid = content.find("## mid-tool").unwrap();
        let pos_low = content.find("## low-tool").unwrap();
        assert!(pos_high < pos_mid, "high-tool should appear before mid-tool");
        assert!(pos_mid < pos_low, "mid-tool should appear before low-tool");
    }

    #[tokio::test]
    async fn empty_results_writes_no_results_message() {
        let dir = tempfile::tempdir().unwrap();
        let reporter = MarkdownReporter::new(dir.path().to_path_buf());

        reporter.report(&[]).await.unwrap();

        let entries: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert_eq!(entries.len(), 1);

        let content = std::fs::read_to_string(entries[0].path()).unwrap();
        assert!(content.contains("# Repo Radar Report"));
        assert!(content.contains("No results found."));
    }

    #[tokio::test]
    async fn nested_nonexistent_output_dir_created() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a").join("b").join("c");
        assert!(!nested.exists());

        let reporter = MarkdownReporter::new(nested.clone());
        reporter.report(&[]).await.unwrap();

        assert!(nested.exists(), "nested directory should have been created");
        let entries: Vec<_> = std::fs::read_dir(&nested)
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert_eq!(entries.len(), 1);
    }
}
