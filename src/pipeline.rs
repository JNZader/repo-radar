use tracing::{info, instrument};

use crate::domain::analyzer::Analyzer;
use crate::domain::crossref::CrossRef;
use crate::domain::filter::Filter;
use crate::domain::reporter::Reporter;
use crate::domain::source::Source;
use crate::infra::error::PipelineError;
use crate::infra::seen::SeenStore;

/// Summary statistics returned after a pipeline run.
#[derive(Debug, Clone)]
pub struct PipelineReport {
    pub entries_fetched: usize,
    pub entries_new: usize,
    pub candidates_filtered: usize,
    pub analyzed: usize,
    pub crossrefed: usize,
    pub reported: usize,
}

impl std::fmt::Display for PipelineReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Pipeline complete: {} fetched, {} new, {} filtered, {} analyzed, {} cross-referenced, {} reported",
            self.entries_fetched,
            self.entries_new,
            self.candidates_filtered,
            self.analyzed,
            self.crossrefed,
            self.reported,
        )
    }
}

/// Orchestrates the full discovery pipeline: fetch → dedupe → filter → analyze → crossref → report.
pub struct Pipeline<S, F, A, X, R>
where
    S: Source,
    F: Filter,
    A: Analyzer,
    X: CrossRef,
    R: Reporter,
{
    source: S,
    filter: F,
    analyzer: A,
    crossref: X,
    reporter: R,
    seen: SeenStore,
}

impl<S, F, A, X, R> Pipeline<S, F, A, X, R>
where
    S: Source,
    F: Filter,
    A: Analyzer,
    X: CrossRef,
    R: Reporter,
{
    pub fn new(source: S, filter: F, analyzer: A, crossref: X, reporter: R, seen: SeenStore) -> Self {
        Self {
            source,
            filter,
            analyzer,
            crossref,
            reporter,
            seen,
        }
    }

    /// Execute the full pipeline: fetch -> dedupe -> filter -> analyze -> crossref -> report.
    ///
    /// # Errors
    ///
    /// Returns `PipelineError` if any stage fails. Subsequent stages are not executed.
    #[instrument(skip_all, name = "pipeline")]
    pub async fn run(&mut self) -> Result<PipelineReport, PipelineError> {
        info!("fetching entries from source");
        let entries = self.source.fetch().await?;
        let entries_fetched = entries.len();
        info!(count = entries_fetched, "entries fetched");

        // Deduplicate against seen store
        let new_entries: Vec<_> = entries
            .into_iter()
            .filter(|e| !self.seen.is_seen(e.repo_url.as_str()))
            .collect();
        let entries_new = new_entries.len();
        info!(count = entries_new, "new entries (not previously seen)");

        info!("filtering candidates");
        let candidates = self.filter.filter(new_entries).await?;
        let candidates_filtered = candidates.len();
        info!(count = candidates_filtered, "candidates after filter");

        info!("analyzing candidates");
        let analyzed = self.analyzer.analyze(candidates).await?;
        let analyzed_count = analyzed.len();
        info!(count = analyzed_count, "analyzed results");

        info!("cross-referencing");
        let crossrefed = self.crossref.cross_reference(analyzed).await?;
        let crossrefed_count = crossrefed.len();
        info!(count = crossrefed_count, "cross-referenced results");

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

        Ok(PipelineReport {
            entries_fetched,
            entries_new,
            candidates_filtered,
            analyzed: analyzed_count,
            crossrefed: crossrefed_count,
            reported,
        })
    }
}
