use std::future::Future;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::domain::model::{KbAnalysis, RepoCandidate};
use crate::infra::error::KbError;

/// Port for persisting and querying the knowledge base.
pub trait KnowledgeBase: Send + Sync {
    /// Insert or replace the analysis for a repo.
    fn upsert_repo(
        &self,
        candidate: &RepoCandidate,
        analysis: KbAnalysis,
    ) -> impl Future<Output = Result<(), KbError>> + Send;

    /// Returns `true` when the repo needs (re-)analysis.
    ///
    /// A repo needs analysis when no record exists, or when `pushed_at` has
    /// changed since the last analysis (i.e. new commits were pushed).
    fn needs_analysis(
        &self,
        owner: &str,
        repo_name: &str,
        pushed_at: Option<DateTime<Utc>>,
    ) -> impl Future<Output = Result<bool, KbError>> + Send;

    /// Full-text search against stored KB entries.
    ///
    /// Returns at most `limit` results ordered by FTS rank.
    fn search(
        &self,
        query: &str,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<KbSearchResult>, KbError>> + Send;
}

/// Port for generating structured LLM analysis from repository content.
pub trait KbAnalyzer: Send + Sync {
    /// Analyze exported repository content and return structured `KbAnalysis`.
    fn analyze(
        &self,
        repo_context: &str,
        owner: &str,
        repo_name: &str,
    ) -> impl Future<Output = Result<KbAnalysis, KbError>> + Send;
}

/// A single full-text search result from the knowledge base.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KbSearchResult {
    pub owner: String,
    pub repo_name: String,
    /// FTS5 snippet highlighting the matched terms.
    pub snippet: String,
    /// FTS5 rank score (lower = more relevant in SQLite FTS5 convention).
    pub rank: f64,
}
