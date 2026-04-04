use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tracing::{info, instrument};

use crate::domain::analyzer::Analyzer;
use crate::domain::categorizer::Categorizer;
use crate::domain::crossref::CrossRef;
use crate::domain::filter::Filter;
use crate::domain::reporter::Reporter;
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
        }
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

        self.emit_progress("analyze", 55, "Analyzing repos...");
        info!("analyzing candidates");
        let analyzed = self.analyzer.analyze(candidates).await?;
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
}
