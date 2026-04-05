#![allow(clippy::manual_async_fn)]

use std::future::Future;

use owo_colors::OwoColorize;

use crate::domain::model::CrossRefResult;
use crate::domain::reporter::Reporter;
use crate::infra::error::ReporterError;

/// A reporter that prints a colored table to the console.
#[derive(Debug, Clone)]
pub struct ConsoleReporter;

impl ConsoleReporter {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ConsoleReporter {
    fn default() -> Self {
        Self::new()
    }
}

impl Reporter for ConsoleReporter {
    fn report(
        &self,
        results: &[CrossRefResult],
    ) -> impl Future<Output = Result<(), ReporterError>> + Send {
        async move {
            if results.is_empty() {
                println!("{}", "No results to display".yellow());
                return Ok(());
            }

            println!(
                "\n{:─<80}",
                "".bold()
            );
            println!(
                " {:<30} {:>8} {:<15} {:<12} {:>6} {:>8}",
                "Repository".bold().cyan(),
                "Stars".bold().cyan(),
                "Language".bold().cyan(),
                "Category".bold().cyan(),
                "Score".bold().cyan(),
                "Matches".bold().cyan(),
            );
            println!("{:─<80}", "".bold());

            for result in results {
                let repo_name = &result.analysis.candidate.repo_name;
                let stars = result.analysis.candidate.stars;
                let language = result
                    .analysis
                    .candidate
                    .language
                    .as_deref()
                    .unwrap_or("N/A");
                let category = result.analysis.candidate.category.to_string();
                let score = result.overall_relevance;
                let match_count = result.matched_repos.len();

                let score_colored = if score >= 0.8 {
                    format!("{score:.2}").green().to_string()
                } else if score >= 0.5 {
                    format!("{score:.2}").yellow().to_string()
                } else {
                    format!("{score:.2}").red().to_string()
                };

                println!(
                    " {:<30} {:>8} {:<15} {:<12} {:>6} {:>8}",
                    repo_name.white(),
                    stars.to_string().bright_yellow(),
                    language.magenta(),
                    category.bright_blue(),
                    score_colored,
                    match_count.to_string().bright_white(),
                );
            }

            println!("{:─<80}", "".bold());
            println!(
                " {} results displayed\n",
                results.len().to_string().bold()
            );

            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use url::Url;

    use super::*;
    use crate::domain::model::{
        AnalysisResult, FeedEntry, RepoCandidate, RepoMatch,
    };

    fn sample_crossref_result() -> CrossRefResult {
        CrossRefResult {
            analysis: AnalysisResult {
                candidate: RepoCandidate {
                    entry: FeedEntry {
                        title: "awesome-tool".into(),
                        repo_url: Url::parse("https://github.com/owner/awesome-tool")
                            .unwrap(),
                        description: Some("A great tool".into()),
                        published: Some(Utc::now()),
                        source_name: "GitHub Trending".into(),
                    },
                    stars: 1234,
                    language: Some("Rust".into()),
                    topics: vec!["cli".into()],
                    fork: false,
                    archived: false,
                    owner: "owner".into(),
                    repo_name: "awesome-tool".into(),
                    category: Default::default(),
                    semantic_score: 0.0,
                },
                summary: "An awesome tool for Rust developers".into(),
                key_features: vec!["fast".into(), "safe".into()],
                tech_stack: vec!["Rust".into(), "tokio".into()],
                relevance_score: 0.85,
            },
            matched_repos: vec![RepoMatch {
                own_repo: "my-project".into(),
                relevance: 0.75,
                reason: "Similar tech stack".into(),
            }],
            ideas: vec!["Integrate this pattern".into()],
            overall_relevance: 0.82,
        }
    }

    #[tokio::test]
    async fn report_happy_path_returns_ok() {
        let reporter = ConsoleReporter::new();
        let results = vec![sample_crossref_result()];

        let result = reporter.report(&results).await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn report_empty_results_returns_ok() {
        let reporter = ConsoleReporter::new();
        let results: Vec<CrossRefResult> = vec![];

        let result = reporter.report(&results).await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn report_multiple_results_returns_ok() {
        let reporter = ConsoleReporter::new();
        let mut result1 = sample_crossref_result();
        result1.overall_relevance = 0.9;

        let mut result2 = sample_crossref_result();
        result2.analysis.candidate.repo_name = "another-tool".into();
        result2.analysis.candidate.language = None;
        result2.overall_relevance = 0.3;
        result2.matched_repos = vec![];

        let results = vec![result1, result2];

        let result = reporter.report(&results).await;

        assert!(result.is_ok());
    }

    #[test]
    fn console_reporter_default_matches_new() {
        let _reporter: ConsoleReporter = ConsoleReporter;
        // Ensuring Default trait works — no panic is the assertion.
    }
}
