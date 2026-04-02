use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use url::Url;

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

#[cfg(test)]
mod tests {
    use super::*;

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
        };
        let json = serde_json::to_string(&candidate).unwrap();
        let deserialized: RepoCandidate = serde_json::from_str(&json).unwrap();
        assert_eq!(candidate, deserialized);
        assert!(deserialized.language.is_none());
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
}
