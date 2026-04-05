use std::cmp::Ordering;

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tracing::{info, instrument};

use crate::config::AnalyzerConfig;
use crate::domain::analyzer::Analyzer;
use crate::domain::categorizer::Categorizer;
use crate::domain::crossref::CrossRef;
use crate::domain::filter::Filter;
use crate::domain::model::OwnRepoSummary;
use crate::domain::reporter::Reporter;
use crate::domain::scorer::semantic_score;
use crate::domain::source::Source;
use crate::infra::error::PipelineError;
use crate::infra::seen::SeenStore;

/// Progress update emitted during a scan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanProgress {
    pub stage: String,
    pub percent: u8,
    pub message: String,
}

/// Summary statistics returned after a pipeline run.
#[derive(Debug, Clone)]
pub struct PipelineReport {
    pub entries_fetched: usize,
    pub entries_new: usize,
    pub candidates_filtered: usize,
    pub categorized: usize,
    pub analyzed: usize,
    pub crossrefed: usize,
    pub reported: usize,
}

impl std::fmt::Display for PipelineReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Pipeline complete: {} fetched, {} new, {} filtered, {} categorized, {} analyzed, {} cross-referenced, {} reported",
            self.entries_fetched,
            self.entries_new,
            self.candidates_filtered,
            self.categorized,
            self.analyzed,
            self.crossrefed,
            self.reported,
        )
    }
}

/// Orchestrates the full discovery pipeline: fetch → dedupe → filter → categorize → analyze → crossref → report.
pub struct Pipeline<S, F, C, A, X, R>
where
    S: Source,
    F: Filter,
    C: Categorizer,
    A: Analyzer,
    X: CrossRef,
    R: Reporter,
{
    source: S,
    filter: F,
    categorizer: C,
    analyzer: A,
    crossref: X,
    reporter: R,
    seen: SeenStore,
    progress_tx: Option<broadcast::Sender<ScanProgress>>,
    /// Analyzer config — controls deep-analysis top-N and min relevance thresholds.
    analyzer_config: AnalyzerConfig,
    /// Pre-fetched summaries of the user's own repos for semantic pre-scoring.
    /// When `None`, semantic scoring is skipped and all candidates go to deep analysis.
    own_repos: Option<Vec<OwnRepoSummary>>,
}

impl<S, F, C, A, X, R> Pipeline<S, F, C, A, X, R>
where
    S: Source,
    F: Filter,
    C: Categorizer,
    A: Analyzer,
    X: CrossRef,
    R: Reporter,
{
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        source: S,
        filter: F,
        categorizer: C,
        analyzer: A,
        crossref: X,
        reporter: R,
        seen: SeenStore,
        progress_tx: Option<broadcast::Sender<ScanProgress>>,
    ) -> Self {
        Self {
            source,
            filter,
            categorizer,
            analyzer,
            crossref,
            reporter,
            seen,
            progress_tx,
            analyzer_config: AnalyzerConfig::default(),
            own_repos: None,
        }
    }

    /// Attach analyzer config (controls deep-analysis top-N and min relevance).
    pub fn with_analyzer_config(mut self, config: AnalyzerConfig) -> Self {
        self.analyzer_config = config;
        self
    }

    /// Attach pre-fetched own-repo summaries for semantic pre-scoring.
    pub fn with_own_repos(mut self, repos: Vec<OwnRepoSummary>) -> Self {
        self.own_repos = Some(repos);
        self
    }

    /// Send a progress event if a channel is configured. Silently ignores if None.
    fn emit_progress(&self, stage: &str, percent: u8, message: &str) {
        if let Some(ref tx) = self.progress_tx {
            // Ignore send errors (no active receivers is fine)
            let _ = tx.send(ScanProgress {
                stage: stage.to_string(),
                percent,
                message: message.to_string(),
            });
        }
    }

    /// Execute the full pipeline: fetch -> dedupe -> filter -> analyze -> crossref -> report.
    ///
    /// Returns a tuple of `(PipelineReport, Vec<CrossRefResult>)` so callers
    /// can persist the results independently of the reporter stage.
    ///
    /// # Errors
    ///
    /// Returns `PipelineError` if any stage fails. Subsequent stages are not executed.
    #[instrument(skip_all, name = "pipeline")]
    pub async fn run(
        &mut self,
    ) -> Result<(PipelineReport, Vec<crate::domain::model::CrossRefResult>), PipelineError> {
        self.emit_progress("fetch", 10, "Fetching feeds...");
        info!("fetching entries from source");
        let entries = self.source.fetch().await?;
        let entries_fetched = entries.len();
        info!(count = entries_fetched, "entries fetched");

        self.emit_progress("dedupe", 20, "Deduplicating entries...");
        // Deduplicate against seen store
        let new_entries: Vec<_> = entries
            .into_iter()
            .filter(|e| !self.seen.is_seen(e.repo_url.as_str()))
            .collect();
        let entries_new = new_entries.len();
        info!(count = entries_new, "new entries (not previously seen)");

        self.emit_progress("filter", 35, "Filtering candidates...");
        info!("filtering candidates");
        let candidates = self.filter.filter(new_entries).await?;
        let candidates_filtered = candidates.len();
        info!(count = candidates_filtered, "candidates after filter");

        self.emit_progress("categorize", 45, "Categorizing repos...");
        info!("categorizing candidates");
        let candidates = self.categorizer.categorize(candidates)?;
        info!(count = candidates.len(), "candidates categorized");

        // Semantic pre-scoring: score each candidate against own repos when available.
        let mut candidates = candidates;
        if let Some(ref own_repos) = self.own_repos {
            info!(own_repo_count = own_repos.len(), "running semantic pre-scoring");
            for candidate in &mut candidates {
                candidate.semantic_score = semantic_score(
                    candidate.entry.description.as_deref(),
                    &candidate.topics,
                    own_repos,
                );
                tracing::debug!(
                    repo = %candidate.entry.repo_url,
                    score = candidate.semantic_score,
                    topics = ?candidate.topics,
                    description = ?candidate.entry.description,
                    "semantic score"
                );
            }
        }

        // Select top-N candidates for deep analysis based on semantic score.
        let candidates_for_analysis = {
            let top_n = self.analyzer_config.deep_analysis_top_n;
            let min_rel = self.analyzer_config.deep_analysis_min_relevance;
            if top_n > 0 && self.own_repos.is_some() {
                let mut scored = candidates.clone();
                scored.sort_by(|a, b| {
                    b.semantic_score
                        .partial_cmp(&a.semantic_score)
                        .unwrap_or(Ordering::Equal)
                });
                let filtered: Vec<_> = scored
                    .into_iter()
                    .filter(|c| c.semantic_score >= min_rel)
                    .take(top_n)
                    .collect();
                info!(
                    total_candidates = candidates.len(),
                    selected_for_deep = filtered.len(),
                    top_n,
                    min_relevance = min_rel,
                    "selected candidates for deep analysis"
                );
                filtered
            } else {
                candidates.clone()
            }
        };

        self.emit_progress("analyze", 55, "Analyzing repos...");
        info!("analyzing candidates");
        let analyzed = self.analyzer.analyze(candidates_for_analysis).await?;
        let analyzed_count = analyzed.len();
        info!(count = analyzed_count, "analyzed results");

        self.emit_progress("crossref", 80, "Cross-referencing...");
        info!("cross-referencing");
        let crossrefed = self.crossref.cross_reference(analyzed).await?;
        let crossrefed_count = crossrefed.len();
        info!(count = crossrefed_count, "cross-referenced results");

        self.emit_progress("report", 90, "Generating report...");
        info!("generating report");
        self.reporter.report(&crossrefed).await?;
        let reported = crossrefed_count;
        info!("report generated");

        // Mark all processed entries as seen
        for result in &crossrefed {
            self.seen
                .mark_seen(result.analysis.candidate.entry.repo_url.as_str());
        }
        self.seen.save()?;
        info!(total_seen = self.seen.len(), "seen store saved");

        self.emit_progress("complete", 100, "Scan complete");

        let report = PipelineReport {
            entries_fetched,
            entries_new,
            candidates_filtered,
            categorized: candidates_filtered,
            analyzed: analyzed_count,
            crossrefed: crossrefed_count,
            reported,
        };

        Ok((report, crossrefed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipeline_report_display_shows_all_counts() {
        let report = PipelineReport {
            entries_fetched: 100,
            entries_new: 80,
            candidates_filtered: 50,
            categorized: 50,
            analyzed: 40,
            crossrefed: 30,
            reported: 25,
        };
        let output = format!("{report}");
        assert!(output.contains("100"));
        assert!(output.contains("80"));
        assert!(output.contains("50"));
        assert!(output.contains("40"));
        assert!(output.contains("30"));
        assert!(output.contains("25"));
        assert!(output.contains("fetched"));
        assert!(output.contains("new"));
        assert!(output.contains("filtered"));
        assert!(output.contains("categorized"));
        assert!(output.contains("analyzed"));
        assert!(output.contains("cross-referenced"));
        assert!(output.contains("reported"));
    }

    #[test]
    fn pipeline_report_default_values() {
        let report = PipelineReport {
            entries_fetched: 0,
            entries_new: 0,
            candidates_filtered: 0,
            categorized: 0,
            analyzed: 0,
            crossrefed: 0,
            reported: 0,
        };
        assert_eq!(report.entries_fetched, 0);
        assert_eq!(report.entries_new, 0);
        assert_eq!(report.candidates_filtered, 0);
        assert_eq!(report.categorized, 0);
        assert_eq!(report.analyzed, 0);
        assert_eq!(report.crossrefed, 0);
        assert_eq!(report.reported, 0);

        let output = format!("{report}");
        assert!(output.contains("0 fetched"));
    }

    #[tokio::test]
    async fn pipeline_emits_progress_events() {
        use crate::adapters::analyzer::NoopAnalyzer;
        use crate::adapters::categorizer::NoopCategorizer;
        use crate::adapters::crossref::NoopCrossRef;
        use crate::adapters::filter::NoopFilter;
        use crate::adapters::reporter::NoopReporter;
        use crate::adapters::source::NoopSource;

        let dir = tempfile::tempdir().unwrap();
        let seen_path = dir.path().join("seen.json");
        let seen = SeenStore::load(&seen_path).unwrap();

        let (tx, mut rx) = broadcast::channel(16);

        let mut pipeline = Pipeline::new(
            NoopSource,
            NoopFilter,
            NoopCategorizer,
            NoopAnalyzer,
            NoopCrossRef,
            NoopReporter,
            seen,
            Some(tx),
        );

        let _ = pipeline.run().await.unwrap();

        let mut stages = Vec::new();
        while let Ok(progress) = rx.try_recv() {
            stages.push((progress.stage, progress.percent, progress.message));
        }

        let stage_names: Vec<&str> = stages.iter().map(|(s, _, _)| s.as_str()).collect();
        assert_eq!(
            stage_names,
            vec!["fetch", "dedupe", "filter", "categorize", "analyze", "crossref", "report", "complete"]
        );

        // Verify percentages are monotonically increasing
        let percents: Vec<u8> = stages.iter().map(|(_, p, _)| *p).collect();
        assert_eq!(percents, vec![10, 20, 35, 45, 55, 80, 90, 100]);
    }

    #[tokio::test]
    async fn pipeline_works_without_progress_channel() {
        use crate::adapters::analyzer::NoopAnalyzer;
        use crate::adapters::categorizer::NoopCategorizer;
        use crate::adapters::crossref::NoopCrossRef;
        use crate::adapters::filter::NoopFilter;
        use crate::adapters::reporter::NoopReporter;
        use crate::adapters::source::NoopSource;

        let dir = tempfile::tempdir().unwrap();
        let seen_path = dir.path().join("seen.json");
        let seen = SeenStore::load(&seen_path).unwrap();

        let mut pipeline = Pipeline::new(
            NoopSource,
            NoopFilter,
            NoopCategorizer,
            NoopAnalyzer,
            NoopCrossRef,
            NoopReporter,
            seen,
            None,
        );

        let (report, results) = pipeline.run().await.unwrap();

        assert_eq!(report.entries_fetched, 0);
        assert_eq!(report.entries_new, 0);
        assert_eq!(report.candidates_filtered, 0);
        assert_eq!(report.analyzed, 0);
        assert_eq!(report.crossrefed, 0);
        assert_eq!(report.reported, 0);
        assert!(results.is_empty());
    }

    // ── failing adapters for error-path tests ─────────────────────────────────

    use crate::domain::filter::Filter;
    use crate::domain::model::{FeedEntry, RepoCandidate};
    use crate::domain::source::Source;
    use crate::infra::error::{FilterError, SourceError};

    struct FailingSource;
    impl Source for FailingSource {
        fn fetch(&self) -> impl std::future::Future<Output = Result<Vec<FeedEntry>, SourceError>> + Send {
            async { Err(SourceError::ParseFailed("source boom".into())) }
        }
        fn name(&self) -> &'static str { "failing-source" }
    }

    struct FailingFilter;
    impl Filter for FailingFilter {
        fn filter(&self, _entries: Vec<FeedEntry>) -> impl std::future::Future<Output = Result<Vec<RepoCandidate>, FilterError>> + Send {
            async { Err(FilterError::GitHubApi("filter boom".into())) }
        }
    }

    /// A source that returns N feed entries with valid-enough URLs for dedup.
    struct CountingSource(usize);
    impl Source for CountingSource {
        fn fetch(&self) -> impl std::future::Future<Output = Result<Vec<FeedEntry>, SourceError>> + Send {
            use chrono::Utc;
            use url::Url;
            use crate::domain::model::FeedEntry;

            let count = self.0;
            async move {
                let entries = (0..count)
                    .map(|i| FeedEntry {
                        title: format!("repo-{i}"),
                        repo_url: Url::parse(&format!("https://github.com/owner/repo-{i}")).unwrap(),
                        description: None,
                        published: Some(Utc::now()),
                        source_name: "test".into(),
                    })
                    .collect();
                Ok(entries)
            }
        }
        fn name(&self) -> &'static str { "counting-source" }
    }

    #[tokio::test]
    async fn pipeline_fails_when_source_errors() {
        use crate::adapters::analyzer::NoopAnalyzer;
        use crate::adapters::categorizer::NoopCategorizer;
        use crate::adapters::crossref::NoopCrossRef;
        use crate::adapters::reporter::NoopReporter;

        let dir = tempfile::tempdir().unwrap();
        let seen = SeenStore::load(&dir.path().join("seen.json")).unwrap();

        let mut pipeline = Pipeline::new(
            FailingSource,
            FailingFilter,
            NoopCategorizer,
            NoopAnalyzer,
            NoopCrossRef,
            NoopReporter,
            seen,
            None,
        );

        let result = pipeline.run().await;
        assert!(result.is_err(), "pipeline should fail when source errors");
        let err = result.unwrap_err();
        assert!(
            matches!(err, PipelineError::Source(_)),
            "expected PipelineError::Source, got: {err}"
        );
    }

    #[tokio::test]
    async fn pipeline_fails_when_filter_errors() {
        use crate::adapters::analyzer::NoopAnalyzer;
        use crate::adapters::categorizer::NoopCategorizer;
        use crate::adapters::crossref::NoopCrossRef;
        use crate::adapters::reporter::NoopReporter;

        let dir = tempfile::tempdir().unwrap();
        let seen = SeenStore::load(&dir.path().join("seen.json")).unwrap();

        // CountingSource produces entries so filter actually receives them → fails.
        let mut pipeline = Pipeline::new(
            CountingSource(3),
            FailingFilter,
            NoopCategorizer,
            NoopAnalyzer,
            NoopCrossRef,
            NoopReporter,
            seen,
            None,
        );
        let result = pipeline.run().await;
        assert!(result.is_err(), "pipeline should fail when filter errors");
        assert!(
            matches!(result.unwrap_err(), PipelineError::Filter(_)),
            "expected PipelineError::Filter"
        );
    }

    #[tokio::test]
    async fn pipeline_skips_seen_entries() {
        use crate::adapters::analyzer::NoopAnalyzer;
        use crate::adapters::categorizer::NoopCategorizer;
        use crate::adapters::crossref::NoopCrossRef;
        use crate::adapters::filter::NoopFilter;
        use crate::adapters::reporter::NoopReporter;

        let dir = tempfile::tempdir().unwrap();
        let seen_path = dir.path().join("seen.json");
        let mut seen = SeenStore::load(&seen_path).unwrap();

        // Pre-mark repo-0 and repo-1 as seen
        seen.mark_seen("https://github.com/owner/repo-0");
        seen.mark_seen("https://github.com/owner/repo-1");
        seen.save().unwrap();

        // Reload to confirm persistence
        let seen = SeenStore::load(&seen_path).unwrap();

        let mut pipeline = Pipeline::new(
            CountingSource(3), // produces repo-0, repo-1, repo-2
            NoopFilter,
            NoopCategorizer,
            NoopAnalyzer,
            NoopCrossRef,
            NoopReporter,
            seen,
            None,
        );

        let (report, _results) = pipeline.run().await.unwrap();

        assert_eq!(report.entries_fetched, 3);
        assert_eq!(report.entries_new, 1, "only repo-2 is new — 0 and 1 are seen");
    }

    #[test]
    fn pipeline_report_display_contains_categorized() {
        let report = PipelineReport {
            entries_fetched: 5,
            entries_new: 5,
            candidates_filtered: 4,
            categorized: 4,
            analyzed: 3,
            crossrefed: 3,
            reported: 3,
        };
        let output = format!("{report}");
        assert!(output.contains("4 categorized"), "display must mention categorized count");
    }
}
