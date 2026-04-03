use std::collections::HashSet;

use chrono::Utc;

use crate::domain::idea_extractor::IdeaExtractor;
use crate::domain::model::{
    CrossRefResult, Idea, IdeaImpact, IdeaKind, IdeaReport,
};
use crate::infra::error::IdeaError;

/// Keyword-based idea extractor that maps crossref results into actionable ideas.
///
/// For each `CrossRefResult` with matched repos, it extracts ideas by comparing
/// key features, tech stack, and categories. Ideas are classified by kind and
/// scored by impact based on overlap strength.
#[derive(Debug, Clone)]
pub struct KeywordIdeaExtractor {
    /// Minimum relevance threshold for a match to generate ideas.
    min_relevance: f64,
}

impl KeywordIdeaExtractor {
    #[must_use]
    pub fn new(min_relevance: f64) -> Self {
        Self { min_relevance }
    }

    /// Classify the idea kind based on what overlaps exist between source and
    /// the crossref match reason.
    fn classify_kind(reason: &str, key_features: &[String], tech_stack: &[String]) -> IdeaKind {
        let reason_lower = reason.to_ascii_lowercase();

        // If the match is primarily about shared technology
        if reason_lower.contains("shared language") && !reason_lower.contains("shared topics") {
            return IdeaKind::TechAdoption;
        }

        // If there are key features that could be adopted
        if !key_features.is_empty() && reason_lower.contains("shared topics") {
            return IdeaKind::FeatureAdoption;
        }

        // If tech stack has items not in the reason (gap to fill)
        if !tech_stack.is_empty() && reason_lower.contains("shared language") {
            return IdeaKind::PatternTransfer;
        }

        // Default to gap fill when topics match but features differ
        IdeaKind::GapFill
    }

    /// Estimate impact based on relevance score and feature richness.
    fn estimate_impact(relevance: f64, feature_count: usize) -> IdeaImpact {
        if relevance >= 0.6 && feature_count >= 2 {
            IdeaImpact::High
        } else if relevance >= 0.3 || feature_count >= 1 {
            IdeaImpact::Medium
        } else {
            IdeaImpact::Low
        }
    }

    /// Build a description for the idea from the available data.
    fn build_description(
        kind: &IdeaKind,
        source_repo: &str,
        target_repo: &str,
        key_features: &[String],
        tech_stack: &[String],
        summary: &str,
    ) -> String {
        match kind {
            IdeaKind::FeatureAdoption => {
                let features = if key_features.is_empty() {
                    "its patterns".to_string()
                } else {
                    key_features.join(", ")
                };
                format!(
                    "Adopt features from {source_repo} into {target_repo}: {features}. {summary}"
                )
            }
            IdeaKind::GapFill => {
                format!(
                    "Fill gaps in {target_repo} using approaches from {source_repo}. {summary}"
                )
            }
            IdeaKind::TechAdoption => {
                let tech = if tech_stack.is_empty() {
                    "its tech stack".to_string()
                } else {
                    tech_stack.join(", ")
                };
                format!(
                    "Consider adopting tech from {source_repo} in {target_repo}: {tech}. {summary}"
                )
            }
            IdeaKind::PatternTransfer => {
                format!(
                    "Transfer architectural patterns from {source_repo} to {target_repo}. {summary}"
                )
            }
        }
    }
}

impl Default for KeywordIdeaExtractor {
    fn default() -> Self {
        Self::new(0.1)
    }
}

impl IdeaExtractor for KeywordIdeaExtractor {
    fn extract(&self, results: &[CrossRefResult]) -> Result<IdeaReport, IdeaError> {
        let mut ideas = Vec::new();
        let mut target_repos: HashSet<String> = HashSet::new();

        for result in results {
            let analysis = &result.analysis;
            let candidate = &analysis.candidate;

            for matched in &result.matched_repos {
                if matched.relevance < self.min_relevance {
                    continue;
                }

                target_repos.insert(matched.own_repo.clone());

                let kind = Self::classify_kind(
                    &matched.reason,
                    &analysis.key_features,
                    &analysis.tech_stack,
                );
                let impact =
                    Self::estimate_impact(matched.relevance, analysis.key_features.len());

                let description = Self::build_description(
                    &kind,
                    &candidate.repo_name,
                    &matched.own_repo,
                    &analysis.key_features,
                    &analysis.tech_stack,
                    &analysis.summary,
                );

                ideas.push(Idea {
                    source_repo: format!("{}/{}", candidate.owner, candidate.repo_name),
                    source_url: candidate.entry.repo_url.clone(),
                    target_repo: matched.own_repo.clone(),
                    description,
                    kind,
                    impact,
                    relevance: matched.relevance,
                    source_features: analysis.key_features.clone(),
                    relevant_tech: analysis.tech_stack.clone(),
                    category: candidate.category.clone(),
                });
            }
        }

        // Sort by relevance descending
        ideas.sort_by(|a, b| {
            b.relevance
                .partial_cmp(&a.relevance)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let total_ideas = ideas.len();

        Ok(IdeaReport {
            generated_at: Utc::now(),
            total_ideas,
            repos_analyzed: results.len(),
            target_repos_involved: target_repos.len(),
            ideas,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::model::{
        AnalysisResult, CrossRefResult, FeedEntry, RepoCandidate, RepoCategory, RepoMatch,
    };
    use chrono::Utc;
    use url::Url;

    fn sample_crossref(
        repo_name: &str,
        language: Option<&str>,
        topics: &[&str],
        features: &[&str],
        tech: &[&str],
        matches: Vec<RepoMatch>,
    ) -> CrossRefResult {
        CrossRefResult {
            analysis: AnalysisResult {
                candidate: RepoCandidate {
                    entry: FeedEntry {
                        title: repo_name.into(),
                        repo_url: Url::parse(&format!("https://github.com/owner/{repo_name}"))
                            .unwrap(),
                        description: Some(format!("Description of {repo_name}")),
                        published: Some(Utc::now()),
                        source_name: "test".into(),
                    },
                    stars: 500,
                    language: language.map(String::from),
                    topics: topics.iter().map(|t| (*t).to_string()).collect(),
                    fork: false,
                    archived: false,
                    owner: "owner".into(),
                    repo_name: repo_name.into(),
                    category: RepoCategory::default(),
                },
                summary: format!("Summary of {repo_name}"),
                key_features: features.iter().map(|f| (*f).to_string()).collect(),
                tech_stack: tech.iter().map(|t| (*t).to_string()).collect(),
                relevance_score: 0.8,
            },
            matched_repos: matches.clone(),
            ideas: vec![],
            overall_relevance: if matches.is_empty() {
                0.0
            } else {
                matches.iter().map(|m| m.relevance).sum::<f64>() / matches.len() as f64
            },
        }
    }

    #[test]
    fn extract_empty_results_returns_empty_report() {
        let extractor = KeywordIdeaExtractor::default();
        let report = extractor.extract(&[]).unwrap();
        assert_eq!(report.total_ideas, 0);
        assert_eq!(report.repos_analyzed, 0);
        assert_eq!(report.target_repos_involved, 0);
        assert!(report.ideas.is_empty());
    }

    #[test]
    fn extract_no_matches_returns_empty_ideas() {
        let extractor = KeywordIdeaExtractor::default();
        let result = sample_crossref("test-repo", Some("Rust"), &["cli"], &["fast"], &["tokio"], vec![]);
        let report = extractor.extract(&[result]).unwrap();
        assert_eq!(report.total_ideas, 0);
        assert_eq!(report.repos_analyzed, 1);
        assert_eq!(report.target_repos_involved, 0);
    }

    #[test]
    fn extract_generates_ideas_from_matches() {
        let extractor = KeywordIdeaExtractor::default();
        let result = sample_crossref(
            "cool-tool",
            Some("Rust"),
            &["cli", "tooling"],
            &["fast", "safe"],
            &["Rust", "tokio"],
            vec![RepoMatch {
                own_repo: "my-cli".into(),
                relevance: 0.7,
                reason: "shared language: Rust; shared topics: cli".into(),
            }],
        );

        let report = extractor.extract(&[result]).unwrap();
        assert_eq!(report.total_ideas, 1);
        assert_eq!(report.target_repos_involved, 1);

        let idea = &report.ideas[0];
        assert_eq!(idea.source_repo, "owner/cool-tool");
        assert_eq!(idea.target_repo, "my-cli");
        assert_eq!(idea.kind, IdeaKind::FeatureAdoption);
        assert_eq!(idea.impact, IdeaImpact::High);
        assert!(idea.description.contains("cool-tool"));
        assert!(idea.description.contains("my-cli"));
    }

    #[test]
    fn extract_filters_below_min_relevance() {
        let extractor = KeywordIdeaExtractor::new(0.5);
        let result = sample_crossref(
            "low-match",
            Some("Go"),
            &[],
            &[],
            &["Go"],
            vec![RepoMatch {
                own_repo: "my-project".into(),
                relevance: 0.2,
                reason: "shared language: Go".into(),
            }],
        );

        let report = extractor.extract(&[result]).unwrap();
        assert_eq!(report.total_ideas, 0);
    }

    #[test]
    fn extract_sorts_by_relevance_descending() {
        let extractor = KeywordIdeaExtractor::default();
        let result = sample_crossref(
            "multi-match",
            Some("Rust"),
            &["cli"],
            &["fast"],
            &["tokio"],
            vec![
                RepoMatch {
                    own_repo: "low-match".into(),
                    relevance: 0.3,
                    reason: "shared language: Rust".into(),
                },
                RepoMatch {
                    own_repo: "high-match".into(),
                    relevance: 0.9,
                    reason: "shared language: Rust; shared topics: cli".into(),
                },
            ],
        );

        let report = extractor.extract(&[result]).unwrap();
        assert_eq!(report.total_ideas, 2);
        assert!(report.ideas[0].relevance > report.ideas[1].relevance);
        assert_eq!(report.ideas[0].target_repo, "high-match");
        assert_eq!(report.ideas[1].target_repo, "low-match");
    }

    #[test]
    fn extract_tech_adoption_for_language_only_match() {
        let extractor = KeywordIdeaExtractor::default();
        let result = sample_crossref(
            "lang-tool",
            Some("Rust"),
            &[],
            &[],
            &["tokio"],
            vec![RepoMatch {
                own_repo: "my-project".into(),
                relevance: 0.4,
                reason: "shared language: Rust".into(),
            }],
        );

        let report = extractor.extract(&[result]).unwrap();
        assert_eq!(report.ideas[0].kind, IdeaKind::TechAdoption);
    }

    #[test]
    fn extract_pattern_transfer_for_lang_plus_tech() {
        let extractor = KeywordIdeaExtractor::default();
        let result = sample_crossref(
            "arch-tool",
            Some("Rust"),
            &[],
            &[],
            &["axum", "tokio"],
            vec![RepoMatch {
                own_repo: "my-web-app".into(),
                relevance: 0.5,
                reason: "shared language: Rust; shared topics: web".into(),
            }],
        );

        let report = extractor.extract(&[result]).unwrap();
        // No key_features + shared language + tech_stack = PatternTransfer
        assert_eq!(report.ideas[0].kind, IdeaKind::PatternTransfer);
    }

    #[test]
    fn extract_multiple_results_aggregates_target_repos() {
        let extractor = KeywordIdeaExtractor::default();
        let r1 = sample_crossref(
            "tool-a",
            Some("Rust"),
            &["cli"],
            &["fast"],
            &["tokio"],
            vec![RepoMatch {
                own_repo: "my-cli".into(),
                relevance: 0.7,
                reason: "shared topics: cli".into(),
            }],
        );
        let r2 = sample_crossref(
            "tool-b",
            Some("Rust"),
            &["web"],
            &["async"],
            &["axum"],
            vec![RepoMatch {
                own_repo: "my-web".into(),
                relevance: 0.6,
                reason: "shared topics: web".into(),
            }],
        );

        let report = extractor.extract(&[r1, r2]).unwrap();
        assert_eq!(report.total_ideas, 2);
        assert_eq!(report.repos_analyzed, 2);
        assert_eq!(report.target_repos_involved, 2);
    }

    #[test]
    fn impact_estimation_high() {
        assert_eq!(
            KeywordIdeaExtractor::estimate_impact(0.7, 3),
            IdeaImpact::High
        );
    }

    #[test]
    fn impact_estimation_medium() {
        assert_eq!(
            KeywordIdeaExtractor::estimate_impact(0.4, 1),
            IdeaImpact::Medium
        );
    }

    #[test]
    fn impact_estimation_low() {
        assert_eq!(
            KeywordIdeaExtractor::estimate_impact(0.1, 0),
            IdeaImpact::Low
        );
    }

    #[test]
    fn idea_report_serde_round_trip() {
        let extractor = KeywordIdeaExtractor::default();
        let result = sample_crossref(
            "serde-test",
            Some("Rust"),
            &["cli"],
            &["fast", "safe"],
            &["Rust", "serde"],
            vec![RepoMatch {
                own_repo: "my-project".into(),
                relevance: 0.8,
                reason: "shared language: Rust; shared topics: cli".into(),
            }],
        );

        let report = extractor.extract(&[result]).unwrap();
        let json = serde_json::to_string_pretty(&report).unwrap();
        let deserialized: IdeaReport = serde_json::from_str(&json).unwrap();
        assert_eq!(report, deserialized);
    }
}
