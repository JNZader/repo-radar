use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use tracing::{info, warn};

use crate::adapters::kb::{KbAnalyzerAdapter, KnowledgeBaseAdapter};
use crate::domain::kb::{KbAnalyzer, KnowledgeBase};
use crate::domain::model::RepoCandidate;
use crate::infra::repoforge::RepoforgeRunner;

/// Summary statistics returned after a KB accumulation run.
#[derive(Debug, Clone, Default)]
pub struct KbReport {
    /// Total candidates received.
    pub total: usize,
    /// New analyses stored (cache miss → analyzed → stored).
    pub analyzed: usize,
    /// Cache hits skipped (repo already up-to-date in KB).
    pub skipped: usize,
    /// Errors encountered (warned + continued — never aborts).
    pub failed: usize,
}

impl std::fmt::Display for KbReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "KB accumulation: {} total, {} analyzed, {} skipped (cache), {} failed",
            self.total, self.analyzed, self.skipped, self.failed,
        )
    }
}

/// Runs the knowledge-base accumulation pass on pre-filtered candidates.
///
/// This struct is intentionally decoupled from `Pipeline` — it receives
/// already-filtered `Vec<RepoCandidate>` and processes them independently.
///
/// # Processing per candidate
/// 1. Check `kb.needs_analysis(owner, repo_name, pushed_at)` — skip if cached.
/// 2. `git clone --depth 1 --single-branch` into a `TempDir`.
/// 3. Run `repoforge export --compress -q` on the clone.
/// 4. Call `analyzer.analyze(&export, owner, repo_name)`.
/// 5. Call `kb.upsert_repo(candidate, analysis)`.
///
/// Any error → `warn!` + continue (never aborts the accumulation run).
pub struct KbPipeline {
    pub kb: KnowledgeBaseAdapter,
    pub analyzer: KbAnalyzerAdapter,
    pub runner: RepoforgeRunner,
    pub git_path: PathBuf,
}

impl KbPipeline {
    /// Create a new `KbPipeline`.
    ///
    /// * `kb` — enum dispatch adapter for the knowledge base.
    /// * `analyzer` — enum dispatch adapter for the LLM analyzer.
    /// * `runner` — shared `RepoforgeRunner` (path + timeout).
    /// * `git_path` — path to the `git` binary (defaults to `"git"` in `$PATH`).
    #[must_use]
    pub fn new(
        kb: KnowledgeBaseAdapter,
        analyzer: KbAnalyzerAdapter,
        runner: RepoforgeRunner,
        git_path: PathBuf,
    ) -> Self {
        Self {
            kb,
            analyzer,
            runner,
            git_path,
        }
    }

    /// Process all `candidates`, accumulating results in the KB.
    ///
    /// Returns a [`KbReport`] with counts for total / analyzed / skipped / failed.
    /// Errors are warned and skipped — this method always succeeds.
    pub async fn accumulate(&self, candidates: Vec<RepoCandidate>) -> KbReport {
        let total = candidates.len();
        let mut analyzed = 0usize;
        let mut skipped = 0usize;
        let mut failed = 0usize;

        for candidate in &candidates {
            let owner = &candidate.owner;
            let repo_name = &candidate.repo_name;

            // Step 1: cache check
            match self
                .kb
                .needs_analysis(owner, repo_name, candidate.pushed_at)
                .await
            {
                Ok(false) => {
                    info!(
                        repo = %format!("{owner}/{repo_name}"),
                        "kb cache hit — skipping"
                    );
                    skipped += 1;
                    continue;
                }
                Ok(true) => {
                    // proceed to clone + analyze
                }
                Err(e) => {
                    warn!(
                        repo = %format!("{owner}/{repo_name}"),
                        error = %e,
                        "kb needs_analysis check failed — skipping"
                    );
                    failed += 1;
                    continue;
                }
            }

            // Step 2: shallow-clone into TempDir
            let tmp_dir = match tempfile::TempDir::new() {
                Ok(d) => d,
                Err(e) => {
                    warn!(
                        repo = %format!("{owner}/{repo_name}"),
                        error = %e,
                        "failed to create temp dir for clone"
                    );
                    failed += 1;
                    continue;
                }
            };

            let repo_url = candidate.entry.repo_url.as_str();
            let clone_future = tokio::process::Command::new(&self.git_path)
                .args(["clone", "--depth", "1", "--single-branch", repo_url])
                .arg(tmp_dir.path())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .output();

            let clone_output = match tokio::time::timeout(self.runner.timeout, clone_future).await
            {
                Ok(Ok(out)) => out,
                Ok(Err(e)) => {
                    warn!(
                        repo = %format!("{owner}/{repo_name}"),
                        error = %e,
                        "git clone I/O error"
                    );
                    failed += 1;
                    continue;
                }
                Err(_) => {
                    warn!(
                        repo = %format!("{owner}/{repo_name}"),
                        "git clone timed out"
                    );
                    failed += 1;
                    continue;
                }
            };

            if !clone_output.status.success() {
                warn!(
                    repo = %format!("{owner}/{repo_name}"),
                    code = clone_output.status.code().unwrap_or(-1),
                    "git clone failed"
                );
                failed += 1;
                continue;
            }

            // Step 3: repoforge export --compress
            let export_content = match self.runner.export(tmp_dir.path()).await {
                Ok(content) => content,
                Err(e) => {
                    warn!(
                        repo = %format!("{owner}/{repo_name}"),
                        error = %e,
                        "repoforge export failed"
                    );
                    failed += 1;
                    continue;
                }
            };

            // Step 4: LLM analysis
            let analysis = match self
                .analyzer
                .analyze(&export_content, owner, repo_name)
                .await
            {
                Ok(a) => a,
                Err(e) => {
                    warn!(
                        repo = %format!("{owner}/{repo_name}"),
                        error = %e,
                        "kb analysis failed"
                    );
                    failed += 1;
                    continue;
                }
            };

            // Step 5: persist to KB
            match self.kb.upsert_repo(candidate, analysis).await {
                Ok(()) => {
                    info!(
                        repo = %format!("{owner}/{repo_name}"),
                        "kb stored"
                    );
                    analyzed += 1;
                }
                Err(e) => {
                    warn!(
                        repo = %format!("{owner}/{repo_name}"),
                        error = %e,
                        "kb upsert failed"
                    );
                    failed += 1;
                }
            }
        }

        KbReport {
            total,
            analyzed,
            skipped,
            failed,
        }
    }
}

// ---------------------------------------------------------------------------
// Helper: build a KbPipeline from config + optional kb_path override
// ---------------------------------------------------------------------------

use crate::adapters::kb::{LlmKbAnalyzer, SqliteKnowledgeBase};
use crate::config::KbConfig;
use crate::infra::error::KbError;

/// Build a `KbPipeline` from `KbConfig`.
///
/// If `kb_path` is `Some`, it overrides `config.db_path`.
/// If `config.repoforge_path` is `None`, the runner uses a zero-timeout Noop path
/// but in practice this path should only be reached when `--accumulate` is set
/// and the repoforge binary is configured.
///
/// Returns `KbError::Sqlite` if SQLite fails to open.
pub fn build_kb_pipeline(
    config: &KbConfig,
    kb_path_override: Option<std::path::PathBuf>,
    repoforge_path: Option<std::path::PathBuf>,
    timeout_secs: u64,
) -> Result<KbPipeline, KbError> {
    let db_path = kb_path_override.unwrap_or_else(|| config.db_path.clone());

    let kb = KnowledgeBaseAdapter::Sqlite(SqliteKnowledgeBase::new(&db_path)?);

    let analyzer = KbAnalyzerAdapter::Llm(LlmKbAnalyzer::new(
        config.llm_gateway_url.clone(),
        config.llm_model.clone(),
        config.llm_auth_token.clone(),
    ));

    let repoforge_bin = repoforge_path.unwrap_or_else(|| PathBuf::from("repoforge"));
    let runner = RepoforgeRunner::new(repoforge_bin, Duration::from_secs(timeout_secs));

    Ok(KbPipeline::new(
        kb,
        analyzer,
        runner,
        PathBuf::from("git"),
    ))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::kb::{LlmKbAnalyzer, NoopKb, NoopKbAnalyzer, SqliteKnowledgeBase};
    use crate::domain::kb::KnowledgeBase;
    use crate::domain::model::{FeedEntry, KbAnalysis, KbAnalysisStatus, RepoCandidate, RepoCategory};
    use chrono::TimeZone;
    use url::Url;

    fn make_candidate(owner: &str, repo: &str) -> RepoCandidate {
        RepoCandidate {
            entry: FeedEntry {
                title: repo.to_string(),
                repo_url: Url::parse(&format!("https://github.com/{owner}/{repo}")).unwrap(),
                description: None,
                published: None,
                source_name: "test".into(),
            },
            stars: 0,
            language: None,
            topics: vec![],
            fork: false,
            archived: false,
            owner: owner.to_string(),
            repo_name: repo.to_string(),
            category: RepoCategory::default(),
            semantic_score: 0.0,
            pushed_at: None,
        }
    }

    fn noop_pipeline() -> KbPipeline {
        KbPipeline::new(
            KnowledgeBaseAdapter::Noop(NoopKb),
            KbAnalyzerAdapter::Noop(NoopKbAnalyzer),
            RepoforgeRunner::new(PathBuf::from("false"), Duration::from_secs(5)),
            PathBuf::from("false"),
        )
    }

    #[test]
    fn kb_report_display_shows_all_counts() {
        let report = KbReport {
            total: 10,
            analyzed: 5,
            skipped: 3,
            failed: 2,
        };
        let out = format!("{report}");
        assert!(out.contains("10 total"));
        assert!(out.contains("5 analyzed"));
        assert!(out.contains("3 skipped"));
        assert!(out.contains("2 failed"));
    }

    #[test]
    fn kb_report_default_is_zeros() {
        let r = KbReport::default();
        assert_eq!(r.total, 0);
        assert_eq!(r.analyzed, 0);
        assert_eq!(r.skipped, 0);
        assert_eq!(r.failed, 0);
    }

    #[tokio::test]
    async fn accumulate_empty_candidates_returns_zero_report() {
        let pipeline = noop_pipeline();
        let report = pipeline.accumulate(vec![]).await;
        assert_eq!(report.total, 0);
        assert_eq!(report.analyzed, 0);
        assert_eq!(report.skipped, 0);
        assert_eq!(report.failed, 0);
    }

    #[tokio::test]
    async fn accumulate_with_noop_kb_counts_failures_when_clone_fails() {
        // NoopKb always needs analysis (returns true), but the git and
        // repoforge binaries are "false" (exit 1 immediately) so every
        // candidate should land in `failed`.
        let pipeline = noop_pipeline();
        let candidates = vec![
            make_candidate("owner", "repo-a"),
            make_candidate("owner", "repo-b"),
        ];
        let report = pipeline.accumulate(candidates).await;
        assert_eq!(report.total, 2);
        assert_eq!(report.analyzed, 0);
        assert_eq!(report.skipped, 0);
        // Both failed at git-clone step
        assert_eq!(report.failed, 2);
    }

    #[tokio::test]
    async fn accumulate_empty_returns_zero_skipped() {
        // Verify zero-candidates produces zero skipped (not false positive)
        let pipeline = noop_pipeline();
        let report = pipeline.accumulate(vec![]).await;
        assert_eq!(report.skipped, 0, "no candidates → no skips");
    }

    // ── Phase 6.3: named integration tests ──────────────────────────────────

    /// Returns `(SqliteKnowledgeBase, _guard)` backed by a temp file.
    /// Hold `_guard` alive for the duration of the test.
    fn sqlite_kb_adapter() -> (SqliteKnowledgeBase, tempfile::NamedTempFile) {
        let tmp = tempfile::Builder::new()
            .suffix(".db")
            .tempfile()
            .expect("create temp db file");
        let kb = SqliteKnowledgeBase::new(tmp.path()).expect("open SqliteKnowledgeBase");
        (kb, tmp)
    }

    fn fixture_path(name: &str) -> PathBuf {
        let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        p.push("tests/fixtures");
        p.push(name);
        p
    }

    fn fixture_pipeline(
        kb: KnowledgeBaseAdapter,
        analyzer: KbAnalyzerAdapter,
    ) -> KbPipeline {
        KbPipeline::new(
            kb,
            analyzer,
            RepoforgeRunner::new(fixture_path("fake_repoforge.sh"), Duration::from_secs(10)),
            fixture_path("fake_git.sh"),
        )
    }

    #[tokio::test]
    async fn kb_pipeline_skips_cached_repos() {
        // Upsert 2 candidates into SQLite with matching pushed_at, so
        // needs_analysis returns false and the pipeline skips both.
        let (kb_adapter, _guard) = sqlite_kb_adapter();

        let t = chrono::Utc.with_ymd_and_hms(2025, 6, 1, 0, 0, 0).unwrap();

        // Pre-populate KB so both repos appear as cached
        let mut c1 = make_candidate("owner", "repo-a");
        c1.pushed_at = Some(t);
        let mut c2 = make_candidate("owner", "repo-b");
        c2.pushed_at = Some(t);

        kb_adapter
            .upsert_repo(&c1, KbAnalysis {
                what: "cached".into(),
                status: KbAnalysisStatus::Complete,
                ..Default::default()
            })
            .await
            .expect("pre-upsert repo-a");
        kb_adapter
            .upsert_repo(&c2, KbAnalysis {
                what: "cached".into(),
                status: KbAnalysisStatus::Complete,
                ..Default::default()
            })
            .await
            .expect("pre-upsert repo-b");

        let pipeline = fixture_pipeline(
            KnowledgeBaseAdapter::Sqlite(kb_adapter),
            KbAnalyzerAdapter::Noop(NoopKbAnalyzer),
        );

        let report = pipeline.accumulate(vec![c1, c2]).await;

        assert_eq!(report.total, 2);
        assert_eq!(report.skipped, 2, "both repos must be skipped (cache hit)");
        assert_eq!(report.analyzed, 0);
        assert_eq!(report.failed, 0);
    }

    #[tokio::test]
    async fn kb_pipeline_accumulates_new_repos() {
        // NoopKb (needs_analysis always true) + NoopKbAnalyzer + fake git/repoforge.
        // All 3 candidates should complete the full pipeline and land in `analyzed`.
        let pipeline = fixture_pipeline(
            KnowledgeBaseAdapter::Noop(NoopKb),
            KbAnalyzerAdapter::Noop(NoopKbAnalyzer),
        );

        let candidates = vec![
            make_candidate("owner", "repo-1"),
            make_candidate("owner", "repo-2"),
            make_candidate("owner", "repo-3"),
        ];

        let report = pipeline.accumulate(candidates).await;

        assert_eq!(report.total, 3);
        assert_eq!(report.analyzed, 3, "all 3 repos must be analyzed");
        assert_eq!(report.skipped, 0);
        assert_eq!(report.failed, 0);
    }

    #[tokio::test]
    async fn kb_pipeline_isolates_errors() {
        // NoopKb (needs_analysis=true) + LlmKbAnalyzer backed by wiremock.
        // First request returns HTTP 500 → KbError::LlmRequest → failed += 1.
        // Remaining two return valid JSON → analyzed += 2.
        // Verify pipeline continues after the failure.
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;

        // First LLM call: HTTP 500
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(500).set_body_string("internal error"))
            .up_to_n_times(1)
            .mount(&server)
            .await;

        // Subsequent LLM calls: valid JSON
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{"message": {"content": r#"{"what":"ok","problem":"","architecture":"","techniques":[],"steal":[],"uniqueness":""}"#}}]
            })))
            .mount(&server)
            .await;

        let analyzer = LlmKbAnalyzer::new(server.uri(), "test-model".into(), None);

        let pipeline = fixture_pipeline(
            KnowledgeBaseAdapter::Noop(NoopKb),
            KbAnalyzerAdapter::Llm(analyzer),
        );

        let candidates = vec![
            make_candidate("owner", "fail-repo"),
            make_candidate("owner", "ok-repo-1"),
            make_candidate("owner", "ok-repo-2"),
        ];

        let report = pipeline.accumulate(candidates).await;

        assert_eq!(report.total, 3);
        assert_eq!(report.failed, 1, "first repo must fail due to HTTP 500");
        assert_eq!(report.analyzed, 2, "remaining two repos must succeed");
        assert_eq!(report.skipped, 0);
    }
}
