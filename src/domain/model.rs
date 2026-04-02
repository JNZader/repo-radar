use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use url::Url;

/// A raw entry discovered from a feed source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedEntry {
    pub title: String,
    pub repo_url: Url,
    pub description: Option<String>,
    pub published: Option<DateTime<Utc>>,
    pub source_name: String,
}

/// A feed entry enriched with GitHub metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisResult {
    pub candidate: RepoCandidate,
    pub summary: String,
    pub key_features: Vec<String>,
    pub tech_stack: Vec<String>,
    pub relevance_score: f64,
}

/// Result of cross-referencing against user's own repos.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrossRefResult {
    pub analysis: AnalysisResult,
    pub matched_repos: Vec<RepoMatch>,
    pub ideas: Vec<String>,
    pub overall_relevance: f64,
}

/// A match between a discovered repo and one of the user's repos.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoMatch {
    pub own_repo: String,
    pub relevance: f64,
    pub reason: String,
}
