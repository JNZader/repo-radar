#![allow(clippy::manual_async_fn)] // Trait uses RPITIT pattern, impls must match

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use tokio::task;

use crate::domain::kb::{KbSearchResult, KnowledgeBase};
use crate::domain::model::{KbAnalysis, RepoCandidate};
use crate::infra::error::KbError;
use crate::infra::sqlite_kb::SqliteKb;

/// Adapter that implements the `KnowledgeBase` port by delegating to `SqliteKb`.
///
/// All blocking rusqlite calls are run via `tokio::task::spawn_blocking` so the
/// async executor thread is never blocked.
#[derive(Clone)]
pub struct SqliteKnowledgeBase(Arc<SqliteKb>);

impl SqliteKnowledgeBase {
    /// Open (or create) the SQLite knowledge base at `path`.
    pub fn new(path: &Path) -> Result<Self, KbError> {
        let kb = SqliteKb::open(path)?;
        Ok(Self(Arc::new(kb)))
    }
}

impl KnowledgeBase for SqliteKnowledgeBase {
    fn upsert_repo(
        &self,
        candidate: &RepoCandidate,
        mut analysis: KbAnalysis,
    ) -> impl Future<Output = Result<(), KbError>> + Send {
        // Populate repo metadata from the candidate before persisting.
        analysis.owner = candidate.owner.clone();
        analysis.repo_name = candidate.repo_name.clone();
        analysis.url = candidate.entry.repo_url.to_string();
        analysis.stars = candidate.stars;
        analysis.language = candidate.language.clone();
        analysis.topics = candidate.topics.clone();
        analysis.pushed_at = candidate.pushed_at;

        let inner = Arc::clone(&self.0);
        async move {
            task::spawn_blocking(move || inner.upsert(&analysis))
                .await
                .map_err(|e| KbError::Sqlite(format!("spawn_blocking panicked: {e}")))?
        }
    }

    fn needs_analysis(
        &self,
        owner: &str,
        repo_name: &str,
        pushed_at: Option<DateTime<Utc>>,
    ) -> impl Future<Output = Result<bool, KbError>> + Send {
        let inner = Arc::clone(&self.0);
        let owner = owner.to_owned();
        let repo_name = repo_name.to_owned();
        async move {
            task::spawn_blocking(move || inner.needs_analysis(&owner, &repo_name, pushed_at))
                .await
                .map_err(|e| KbError::Sqlite(format!("spawn_blocking panicked: {e}")))?
        }
    }

    fn search(
        &self,
        query: &str,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<KbSearchResult>, KbError>> + Send {
        let inner = Arc::clone(&self.0);
        let query = query.to_owned();
        async move {
            let analyses = task::spawn_blocking(move || inner.search(&query))
                .await
                .map_err(|e| KbError::Sqlite(format!("spawn_blocking panicked: {e}")))??;

            // Map KbAnalysis → KbSearchResult and truncate to `limit`.
            let results = analyses
                .into_iter()
                .take(limit)
                .map(|a| KbSearchResult {
                    owner: a.owner,
                    repo_name: a.repo_name,
                    snippet: a.what,
                    rank: 0.0,
                })
                .collect();
            Ok(results)
        }
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use url::Url;

    use crate::domain::model::{FeedEntry, KbAnalysisStatus, RepoCategory};

    fn make_candidate(owner: &str, repo: &str) -> RepoCandidate {
        RepoCandidate {
            entry: FeedEntry {
                title: repo.to_owned(),
                repo_url: Url::parse(&format!("https://github.com/{owner}/{repo}")).unwrap(),
                description: None,
                published: None,
                source_name: "test".into(),
            },
            stars: 99,
            language: Some("Rust".into()),
            topics: vec!["cli".into()],
            fork: false,
            archived: false,
            owner: owner.to_owned(),
            repo_name: repo.to_owned(),
            category: RepoCategory::default(),
            semantic_score: 0.0,
            pushed_at: Some(Utc.with_ymd_and_hms(2025, 6, 1, 0, 0, 0).unwrap()),
        }
    }

    fn make_analysis(what: &str) -> KbAnalysis {
        KbAnalysis {
            what: what.to_owned(),
            problem: "a problem".into(),
            architecture: "hexagonal".into(),
            techniques: vec!["async".into()],
            steal: vec!["plugin system".into()],
            uniqueness: "unique".into(),
            status: KbAnalysisStatus::Complete,
            ..Default::default()
        }
    }

    /// Returns `(adapter, _guard)` — hold `_guard` alive for the test duration.
    /// Dropping `_guard` removes the temp file, which would invalidate the DB.
    fn in_memory_adapter() -> (SqliteKnowledgeBase, tempfile::NamedTempFile) {
        let tmp = tempfile::Builder::new()
            .suffix(".db")
            .tempfile()
            .expect("create temp db file");
        let adapter = SqliteKnowledgeBase::new(tmp.path()).expect("open SqliteKnowledgeBase");
        (adapter, tmp)
    }

    #[tokio::test]
    async fn upsert_and_needs_analysis_cache_hit() {
        let (kb, _guard) = in_memory_adapter();
        let candidate = make_candidate("owner", "my-repo");
        let analysis = make_analysis("A fast tool");

        kb.upsert_repo(&candidate, analysis).await.unwrap();

        // Same pushed_at → should be a cache hit (no re-analysis needed)
        let needs = kb
            .needs_analysis("owner", "my-repo", candidate.pushed_at)
            .await
            .unwrap();
        assert!(!needs, "same pushed_at must be a cache hit");
    }

    #[tokio::test]
    async fn needs_analysis_true_for_unknown_repo() {
        let (kb, _guard) = in_memory_adapter();
        let needs = kb.needs_analysis("ghost", "repo", None).await.unwrap();
        assert!(needs, "unknown repo must need analysis");
    }

    #[tokio::test]
    async fn needs_analysis_true_when_pushed_at_changes() {
        let (kb, _guard) = in_memory_adapter();
        let old_push = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();
        let new_push = Utc.with_ymd_and_hms(2025, 6, 1, 0, 0, 0).unwrap();

        let mut candidate = make_candidate("owner", "repo");
        candidate.pushed_at = Some(old_push);
        let analysis = make_analysis("Tool");
        kb.upsert_repo(&candidate, analysis).await.unwrap();

        let needs = kb
            .needs_analysis("owner", "repo", Some(new_push))
            .await
            .unwrap();
        assert!(needs, "changed pushed_at must trigger re-analysis");
    }

    #[tokio::test]
    async fn search_returns_matching_entries() {
        let (kb, _guard) = in_memory_adapter();
        let candidate = make_candidate("acme", "migration-tool");
        let analysis = make_analysis("A database migration tool");
        kb.upsert_repo(&candidate, analysis).await.unwrap();

        let results = kb.search("migration", 10).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].owner, "acme");
        assert_eq!(results[0].repo_name, "migration-tool");
    }

    #[tokio::test]
    async fn search_respects_limit() {
        let (kb, _guard) = in_memory_adapter();
        for i in 0..5 {
            let candidate = make_candidate("owner", &format!("repo-{i}"));
            let analysis = make_analysis("pipeline automation tool");
            kb.upsert_repo(&candidate, analysis).await.unwrap();
        }

        let results = kb.search("pipeline", 3).await.unwrap();
        assert!(results.len() <= 3, "limit must be respected");
    }

    #[tokio::test]
    async fn search_returns_empty_on_no_match() {
        let (kb, _guard) = in_memory_adapter();
        let results = kb.search("xyznonexistent", 10).await.unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn upsert_populates_metadata_from_candidate() {
        let (kb, _guard) = in_memory_adapter();
        let candidate = make_candidate("javier", "cool-repo");
        // Analysis has no owner/repo_name set — they should be filled from candidate
        let analysis = make_analysis("A cool thing");
        kb.upsert_repo(&candidate, analysis).await.unwrap();

        let results = kb.search("cool", 10).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].owner, "javier");
        assert_eq!(results[0].repo_name, "cool-repo");
    }
}
