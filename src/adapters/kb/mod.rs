pub mod llm_kb_analyzer;
pub mod sqlite_kb;

pub use llm_kb_analyzer::LlmKbAnalyzer;
pub use sqlite_kb::SqliteKnowledgeBase;

use crate::domain::kb::{KbAnalyzer, KbSearchResult, KnowledgeBase};
use crate::domain::model::{KbAnalysis, RepoCandidate};
use crate::infra::error::KbError;
use chrono::{DateTime, Utc};
use std::future::Future;

// ---------------------------------------------------------------------------
// NoopKb — always needs analysis, never stores, never returns results
// ---------------------------------------------------------------------------

/// No-op knowledge base — useful for tests and dry-run modes.
///
/// * `needs_analysis` always returns `true`
/// * `upsert_repo` is a no-op
/// * `search` always returns `[]`
pub struct NoopKb;

impl KnowledgeBase for NoopKb {
    fn upsert_repo(
        &self,
        _candidate: &RepoCandidate,
        _analysis: KbAnalysis,
    ) -> impl Future<Output = Result<(), KbError>> + Send {
        async { Ok(()) }
    }

    fn needs_analysis(
        &self,
        _owner: &str,
        _repo_name: &str,
        _pushed_at: Option<DateTime<Utc>>,
    ) -> impl Future<Output = Result<bool, KbError>> + Send {
        async { Ok(true) }
    }

    fn search(
        &self,
        _query: &str,
        _limit: usize,
    ) -> impl Future<Output = Result<Vec<KbSearchResult>, KbError>> + Send {
        async { Ok(vec![]) }
    }
}

// ---------------------------------------------------------------------------
// NoopKbAnalyzer — returns a Pending/default KbAnalysis
// ---------------------------------------------------------------------------

use crate::domain::model::KbAnalysisStatus;

/// No-op KB analyzer — returns an empty `KbAnalysis` with `status: Pending`.
///
/// Used in tests and dry-run modes where no LLM is available.
/// Note: `KbAnalysisStatus` has no `Pending` variant by design; this returns
/// `ParseFailed` with an empty raw response to signal "not analyzed".
pub struct NoopKbAnalyzer;

impl KbAnalyzer for NoopKbAnalyzer {
    fn analyze(
        &self,
        _repo_context: &str,
        owner: &str,
        repo_name: &str,
    ) -> impl Future<Output = Result<KbAnalysis, KbError>> + Send {
        let owner = owner.to_owned();
        let repo_name = repo_name.to_owned();
        async move {
            Ok(KbAnalysis {
                owner,
                repo_name,
                status: KbAnalysisStatus::ParseFailed,
                raw_llm_response: None,
                ..Default::default()
            })
        }
    }
}

// ---------------------------------------------------------------------------
// Enum dispatch adapters — KnowledgeBaseAdapter + KbAnalyzerAdapter
// ---------------------------------------------------------------------------

/// Enum dispatch for `KnowledgeBase` — avoids `Box<dyn KnowledgeBase>` in
/// callers that don't need trait-object polymorphism.
pub enum KnowledgeBaseAdapter {
    Sqlite(SqliteKnowledgeBase),
    Noop(NoopKb),
}

impl KnowledgeBase for KnowledgeBaseAdapter {
    fn upsert_repo(
        &self,
        candidate: &RepoCandidate,
        analysis: KbAnalysis,
    ) -> impl Future<Output = Result<(), KbError>> + Send {
        // RPITIT enum dispatch: different match arms produce different anonymous
        // async block types, so we cannot use `match` directly. Instead we extract
        // an `Option<Clone>` before the async block so a single async block handles
        // both paths.
        let candidate = candidate.clone();
        let sqlite = if let Self::Sqlite(inner) = self {
            Some(inner.clone())
        } else {
            None
        };
        async move {
            if let Some(inner) = sqlite {
                inner.upsert_repo(&candidate, analysis).await
            } else {
                Ok(())
            }
        }
    }

    fn needs_analysis(
        &self,
        owner: &str,
        repo_name: &str,
        pushed_at: Option<DateTime<Utc>>,
    ) -> impl Future<Output = Result<bool, KbError>> + Send {
        let owner = owner.to_owned();
        let repo_name = repo_name.to_owned();
        let sqlite = if let Self::Sqlite(inner) = self {
            Some(inner.clone())
        } else {
            None
        };
        async move {
            if let Some(inner) = sqlite {
                inner.needs_analysis(&owner, &repo_name, pushed_at).await
            } else {
                Ok(true)
            }
        }
    }

    fn search(
        &self,
        query: &str,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<KbSearchResult>, KbError>> + Send {
        let query = query.to_owned();
        let sqlite = if let Self::Sqlite(inner) = self {
            Some(inner.clone())
        } else {
            None
        };
        async move {
            if let Some(inner) = sqlite {
                inner.search(&query, limit).await
            } else {
                Ok(vec![])
            }
        }
    }
}

/// Enum dispatch for `KbAnalyzer` — avoids `Box<dyn KbAnalyzer>` in callers
/// that don't need trait-object polymorphism.
pub enum KbAnalyzerAdapter {
    Llm(LlmKbAnalyzer),
    Noop(NoopKbAnalyzer),
}

impl KbAnalyzer for KbAnalyzerAdapter {
    fn analyze(
        &self,
        repo_context: &str,
        owner: &str,
        repo_name: &str,
    ) -> impl Future<Output = Result<KbAnalysis, KbError>> + Send {
        let repo_context = repo_context.to_owned();
        let owner = owner.to_owned();
        let repo_name = repo_name.to_owned();
        // LlmKbAnalyzer is Clone (reqwest::Client is Arc-backed internally).
        let llm = if let Self::Llm(inner) = self {
            Some(inner.clone())
        } else {
            None
        };
        async move {
            if let Some(inner) = llm {
                inner.analyze(&repo_context, &owner, &repo_name).await
            } else {
                Ok(KbAnalysis {
                    owner,
                    repo_name,
                    status: KbAnalysisStatus::ParseFailed,
                    raw_llm_response: None,
                    ..Default::default()
                })
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests for Noop adapters and enum dispatch
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use url::Url;
    use crate::domain::model::{FeedEntry, RepoCategory};

    fn make_candidate() -> RepoCandidate {
        RepoCandidate {
            entry: FeedEntry {
                title: "repo".into(),
                repo_url: Url::parse("https://github.com/owner/repo").unwrap(),
                description: None,
                published: None,
                source_name: "test".into(),
            },
            stars: 0,
            language: None,
            topics: vec![],
            fork: false,
            archived: false,
            owner: "owner".into(),
            repo_name: "repo".into(),
            category: RepoCategory::default(),
            semantic_score: 0.0,
            pushed_at: None,
        }
    }

    #[tokio::test]
    async fn noop_kb_needs_analysis_always_true() {
        let kb = NoopKb;
        assert!(kb.needs_analysis("owner", "repo", None).await.unwrap());
    }

    #[tokio::test]
    async fn noop_kb_upsert_is_noop() {
        let kb = NoopKb;
        let result = kb
            .upsert_repo(&make_candidate(), KbAnalysis::default())
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn noop_kb_search_returns_empty() {
        let kb = NoopKb;
        let results = kb.search("anything", 10).await.unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn noop_analyzer_returns_parse_failed() {
        let analyzer = NoopKbAnalyzer;
        let analysis = analyzer.analyze("ctx", "owner", "repo").await.unwrap();
        assert_eq!(analysis.status, KbAnalysisStatus::ParseFailed);
        assert_eq!(analysis.owner, "owner");
        assert_eq!(analysis.repo_name, "repo");
        assert!(analysis.raw_llm_response.is_none());
    }

    #[tokio::test]
    async fn enum_dispatch_noop_kb_needs_analysis() {
        let adapter = KnowledgeBaseAdapter::Noop(NoopKb);
        assert!(adapter.needs_analysis("owner", "repo", None).await.unwrap());
    }

    #[tokio::test]
    async fn enum_dispatch_noop_kb_search() {
        let adapter = KnowledgeBaseAdapter::Noop(NoopKb);
        assert!(adapter.search("query", 10).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn enum_dispatch_noop_analyzer() {
        let adapter = KbAnalyzerAdapter::Noop(NoopKbAnalyzer);
        let analysis = adapter.analyze("ctx", "owner", "repo").await.unwrap();
        assert_eq!(analysis.status, KbAnalysisStatus::ParseFailed);
    }
}
