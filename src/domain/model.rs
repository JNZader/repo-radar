use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use url::Url;

/// Category assigned to a discovered repository.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum RepoCategory {
    AiAgents,
    Security,
    DevOps,
    RagSearch,
    Memory,
    Documentation,
    Testing,
    UiUx,
    Workflow,
    #[default]
    Other,
}

impl fmt::Display for RepoCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AiAgents => write!(f, "AI Agents"),
            Self::Security => write!(f, "Security"),
            Self::DevOps => write!(f, "DevOps"),
            Self::RagSearch => write!(f, "RAG/Search"),
            Self::Memory => write!(f, "Memory"),
            Self::Documentation => write!(f, "Documentation"),
            Self::Testing => write!(f, "Testing"),
            Self::UiUx => write!(f, "UI/UX"),
            Self::Workflow => write!(f, "Workflow"),
            Self::Other => write!(f, "Other"),
        }
    }
}

/// A raw entry discovered from a feed source.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FeedEntry {
    pub title: String,
    pub repo_url: Url,
    pub description: Option<String>,
    pub published: Option<DateTime<Utc>>,
    pub source_name: String,
}

/// A feed entry enriched with GitHub metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RepoCandidate {
    pub entry: FeedEntry,
    pub stars: u64,
    pub language: Option<String>,
    pub topics: Vec<String>,
    pub fork: bool,
    pub archived: bool,
    pub owner: String,
    pub repo_name: String,
    #[serde(default)]
    pub category: RepoCategory,
    /// Semantic similarity score against the user's own repos (0.0–1.0).
    /// Computed before deep analysis to prioritize candidates.
    #[serde(default)]
    pub semantic_score: f64,
    /// Last push timestamp from GitHub API — used as cache key for KB analysis freshness.
    #[serde(default)]
    pub pushed_at: Option<DateTime<Utc>>,
}

/// A compact representation of one of the user's own repos used for semantic scoring.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OwnRepoSummary {
    pub name: String,
    pub description: Option<String>,
    pub topics: Vec<String>,
}

/// Result of analyzing a repo's content.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnalysisResult {
    pub candidate: RepoCandidate,
    pub summary: String,
    pub key_features: Vec<String>,
    pub tech_stack: Vec<String>,
    pub relevance_score: f64,
}

/// Result of cross-referencing against user's own repos.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CrossRefResult {
    pub analysis: AnalysisResult,
    pub matched_repos: Vec<RepoMatch>,
    pub ideas: Vec<String>,
    pub overall_relevance: f64,
}

/// A match between a discovered repo and one of the user's repos.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RepoMatch {
    pub own_repo: String,
    pub relevance: f64,
    pub reason: String,
}


/// Impact estimate for an extracted idea.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum IdeaImpact {
    High,
    #[default]
    Medium,
    Low,
}

impl fmt::Display for IdeaImpact {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::High => write!(f, "High"),
            Self::Medium => write!(f, "Medium"),
            Self::Low => write!(f, "Low"),
        }
    }
}

/// The kind of actionable idea extracted from a discovered repo.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum IdeaKind {
    #[default]
    FeatureAdoption,
    GapFill,
    TechAdoption,
    PatternTransfer,
}

impl fmt::Display for IdeaKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FeatureAdoption => write!(f, "Feature Adoption"),
            Self::GapFill => write!(f, "Gap Fill"),
            Self::TechAdoption => write!(f, "Tech Adoption"),
            Self::PatternTransfer => write!(f, "Pattern Transfer"),
        }
    }
}

/// An actionable idea extracted by comparing a discovered repo against user repos.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Idea {
    pub source_repo: String,
    pub source_url: Url,
    pub target_repo: String,
    pub description: String,
    pub kind: IdeaKind,
    pub impact: IdeaImpact,
    pub relevance: f64,
    pub source_features: Vec<String>,
    pub relevant_tech: Vec<String>,
    pub category: RepoCategory,
}

/// Status of a KB analysis entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub enum KbAnalysisStatus {
    /// Analysis was successfully parsed and stored.
    #[default]
    Complete,
    /// LLM response could not be parsed; raw response saved in `raw_llm_response`.
    ParseFailed,
}

/// Structured LLM analysis of a repository stored in the knowledge base.
///
/// Combines repository metadata (from GitHub) with LLM-generated analysis fields.
/// All LLM fields use `#[serde(default)]` so partial responses still deserialize cleanly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct KbAnalysis {
    // ── Repository metadata ─────────────────────────────────────────────────
    /// GitHub owner login.
    #[serde(default)]
    pub owner: String,
    /// Repository name (without owner prefix).
    #[serde(default)]
    pub repo_name: String,
    /// Full URL to the repository.
    #[serde(default)]
    pub url: String,
    /// GitHub star count at time of analysis.
    #[serde(default)]
    pub stars: u64,
    /// Primary language reported by GitHub.
    #[serde(default)]
    pub language: Option<String>,
    /// GitHub topic tags.
    #[serde(default)]
    pub topics: Vec<String>,
    /// Last push timestamp from GitHub API (used as cache-freshness key).
    #[serde(default)]
    pub pushed_at: Option<DateTime<Utc>>,
    /// When this repo was first added to the knowledge base.
    #[serde(default = "Utc::now")]
    pub first_seen_at: DateTime<Utc>,
    /// When this repo was last seen/updated.
    #[serde(default = "Utc::now")]
    pub last_seen_at: DateTime<Utc>,
    /// When LLM analysis was last completed successfully.
    #[serde(default)]
    pub analyzed_at: Option<DateTime<Utc>>,

    // ── LLM analysis fields ──────────────────────────────────────────────────
    /// One-line description of what the repo does.
    #[serde(default)]
    pub what: String,
    /// The problem the repo solves.
    #[serde(default)]
    pub problem: String,
    /// Key architectural patterns used.
    #[serde(default)]
    pub architecture: String,
    /// Up to 4 notable techniques (1–4 items).
    #[serde(default)]
    pub techniques: Vec<String>,
    /// Up to 3 ideas worth stealing (1–3 items).
    #[serde(default)]
    pub steal: Vec<String>,
    /// What makes this repo unique or noteworthy.
    #[serde(default)]
    pub uniqueness: String,
    /// Analysis status — `ParseFailed` when LLM response could not be parsed.
    #[serde(default)]
    pub status: KbAnalysisStatus,
    /// Raw LLM response string, populated only on parse failure.
    pub raw_llm_response: Option<String>,
    /// Whether this entry belongs to the user's own repositories.
    /// `false` means it was discovered from external feeds; `true` means it's a
    /// user-owned repo used as the comparison target.
    #[serde(default)]
    pub is_own: bool,
}

impl KbAnalysis {
    /// Return the composite primary key `"owner/repo_name"`.
    pub fn owner_repo_id(&self) -> String {
        format!("{}/{}", self.owner, self.repo_name)
    }
}

/// Collection of extracted ideas with summary metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IdeaReport {
    pub generated_at: DateTime<Utc>,
    pub total_ideas: usize,
    pub repos_analyzed: usize,
    pub target_repos_involved: usize,
    pub ideas: Vec<Idea>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn sample_feed_entry() -> FeedEntry {
        FeedEntry {
            title: "awesome-tool".into(),
            repo_url: Url::parse("https://github.com/owner/awesome-tool").unwrap(),
            description: Some("A great tool".into()),
            published: Some(Utc::now()),
            source_name: "GitHub Trending".into(),
        }
    }

    fn sample_repo_candidate() -> RepoCandidate {
        RepoCandidate {
            entry: sample_feed_entry(),
            stars: 1234,
            language: Some("Rust".into()),
            topics: vec!["cli".into(), "tooling".into()],
            fork: false,
            archived: false,
            owner: "owner".into(),
            repo_name: "awesome-tool".into(),
            category: RepoCategory::default(),
            semantic_score: 0.0,
            pushed_at: None,
        }
    }

    #[test]
    fn feed_entry_serde_round_trip() {
        let entry = sample_feed_entry();
        let json = serde_json::to_string(&entry).unwrap();
        let deserialized: FeedEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(entry, deserialized);
    }

    #[test]
    fn repo_candidate_serde_round_trip() {
        let candidate = sample_repo_candidate();
        let json = serde_json::to_string(&candidate).unwrap();
        let deserialized: RepoCandidate = serde_json::from_str(&json).unwrap();
        assert_eq!(candidate, deserialized);
    }

    #[test]
    fn analysis_result_serde_round_trip() {
        let result = AnalysisResult {
            candidate: sample_repo_candidate(),
            summary: "This tool does amazing things".into(),
            key_features: vec!["fast".into(), "safe".into()],
            tech_stack: vec!["Rust".into(), "tokio".into()],
            relevance_score: 0.85,
        };
        let json = serde_json::to_string(&result).unwrap();
        let deserialized: AnalysisResult = serde_json::from_str(&json).unwrap();
        assert_eq!(result, deserialized);
    }

    #[test]
    fn crossref_result_serde_round_trip() {
        let result = CrossRefResult {
            analysis: AnalysisResult {
                candidate: sample_repo_candidate(),
                summary: "Summary".into(),
                key_features: vec!["feature".into()],
                tech_stack: vec!["Rust".into()],
                relevance_score: 0.9,
            },
            matched_repos: vec![RepoMatch {
                own_repo: "my-project".into(),
                relevance: 0.75,
                reason: "Similar tech stack".into(),
            }],
            ideas: vec!["Integrate this pattern".into()],
            overall_relevance: 0.82,
        };
        let json = serde_json::to_string(&result).unwrap();
        let deserialized: CrossRefResult = serde_json::from_str(&json).unwrap();
        assert_eq!(result, deserialized);
    }

    #[test]
    fn feed_entry_optional_fields_none() {
        let entry = FeedEntry {
            title: "minimal".into(),
            repo_url: Url::parse("https://github.com/a/b").unwrap(),
            description: None,
            published: None,
            source_name: "test".into(),
        };
        let json = serde_json::to_string(&entry).unwrap();
        let deserialized: FeedEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(entry, deserialized);
        assert!(deserialized.description.is_none());
        assert!(deserialized.published.is_none());
    }

    #[test]
    fn repo_candidate_optional_language_none() {
        let candidate = RepoCandidate {
            entry: sample_feed_entry(),
            stars: 0,
            language: None,
            topics: Vec::new(),
            fork: true,
            archived: true,
            owner: "owner".into(),
            repo_name: "repo".into(),
            category: RepoCategory::default(),
            semantic_score: 0.0,
            pushed_at: None,
        };
        let json = serde_json::to_string(&candidate).unwrap();
        let deserialized: RepoCandidate = serde_json::from_str(&json).unwrap();
        assert_eq!(candidate, deserialized);
        assert!(deserialized.language.is_none());
    }

    #[test]
    fn repo_match_serde_round_trip() {
        let m = RepoMatch {
            own_repo: "my-project".into(),
            relevance: 0.88,
            reason: "Shared tech stack".into(),
        };
        let json = serde_json::to_string(&m).unwrap();
        let deserialized: RepoMatch = serde_json::from_str(&json).unwrap();
        assert_eq!(m, deserialized);
    }

    #[test]
    fn feed_entry_clone_is_independent() {
        let original = sample_feed_entry();
        let clone = original.clone();

        // Mutate original via a new binding (structs are public, so rebuild)
        let mutated = FeedEntry {
            title: "changed-title".into(),
            ..original
        };

        assert_ne!(mutated.title, clone.title);
        assert_eq!(clone.title, "awesome-tool");
    }

    #[test]
    fn crossref_result_overall_relevance_zero_with_no_matches() {
        let result = CrossRefResult {
            analysis: AnalysisResult {
                candidate: sample_repo_candidate(),
                summary: "S".into(),
                key_features: vec![],
                tech_stack: vec![],
                relevance_score: 0.0,
            },
            matched_repos: vec![],
            ideas: vec![],
            overall_relevance: 0.0,
        };
        assert!(result.matched_repos.is_empty());
        assert!((result.overall_relevance - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn analysis_result_relevance_score_range() {
        let make = |score: f64| AnalysisResult {
            candidate: sample_repo_candidate(),
            summary: "S".into(),
            key_features: vec![],
            tech_stack: vec![],
            relevance_score: score,
        };

        let zero = make(0.0);
        assert!((zero.relevance_score - 0.0).abs() < f64::EPSILON);

        let one = make(1.0);
        assert!((one.relevance_score - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn crossref_result_empty_matched_repos() {
        let result = CrossRefResult {
            analysis: AnalysisResult {
                candidate: sample_repo_candidate(),
                summary: "S".into(),
                key_features: vec![],
                tech_stack: vec![],
                relevance_score: 0.0,
            },
            matched_repos: vec![],
            ideas: vec![],
            overall_relevance: 0.0,
        };
        let json = serde_json::to_string(&result).unwrap();
        let deserialized: CrossRefResult = serde_json::from_str(&json).unwrap();
        assert_eq!(result, deserialized);
        assert!(deserialized.matched_repos.is_empty());
    }

    proptest! {
        #[test]
        fn prop_feed_entry_serde_round_trip_is_lossless(
            title in "\\PC{1,50}",
            description in proptest::option::of("\\PC{1,100}"),
            source_name in "\\PC{1,30}",
            timestamp in proptest::option::of(proptest::num::i64::ANY),
        ) {
            let published = timestamp.map(|ts| {
                chrono::DateTime::from_timestamp(ts.rem_euclid(4_102_444_800), 0)
                    .unwrap_or_default()
            });
            let entry = FeedEntry {
                title,
                repo_url: Url::parse("https://github.com/test/repo").unwrap(),
                description,
                published,
                source_name,
            };
            let json = serde_json::to_string(&entry).unwrap();
            let deserialized: FeedEntry = serde_json::from_str(&json).unwrap();
            prop_assert_eq!(entry, deserialized);
        }

        #[test]
        fn prop_repo_match_relevance_is_finite(
            relevance in proptest::num::f64::NORMAL | proptest::num::f64::ZERO,
        ) {
            // Test with finite f64 values only (normal + zero); NaN/Inf are
            // invalid JSON numbers and serde_json behaviour for them is
            // implementation-defined, so we focus on the well-defined path.
            let m = RepoMatch {
                own_repo: "test-repo".into(),
                relevance,
                reason: "test reason".into(),
            };
            let json = serde_json::to_string(&m).unwrap();
            let deserialized: RepoMatch = serde_json::from_str(&json).unwrap();
            prop_assert!(deserialized.relevance.is_finite(),
                "finite input must produce finite output");
            if relevance == 0.0 {
                prop_assert_eq!(deserialized.relevance, 0.0);
            } else {
                let rel_error = ((deserialized.relevance - relevance) / relevance).abs();
                prop_assert!(rel_error < 1e-10,
                    "relative error {} too large for value {}", rel_error, relevance);
            }
        }
    }
}
