//! Port contract tests — verify every adapter enum variant satisfies its port trait.

use repo_radar::adapters::analyzer::{AnalyzerAdapter, NoopAnalyzer};
use repo_radar::adapters::crossref::{CrossRefAdapter, NoopCrossRef};
use repo_radar::adapters::filter::{FilterAdapter, NoopFilter};
use repo_radar::adapters::reporter::{ConsoleReporter, NoopReporter, ReporterAdapter};
use repo_radar::adapters::source::{NoopSource, SourceAdapter};
use repo_radar::domain::analyzer::Analyzer;
use repo_radar::domain::crossref::CrossRef;
use repo_radar::domain::filter::Filter;
use repo_radar::domain::reporter::Reporter;
use repo_radar::domain::source::Source;

// ---------------------------------------------------------------------------
// Direct Noop struct tests (trait contract on the concrete type)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn noop_source_returns_empty_vec() {
    let source = NoopSource;
    let result = source.fetch().await.expect("NoopSource.fetch() should succeed");
    assert!(result.is_empty(), "NoopSource must return an empty vec");
}

#[tokio::test]
async fn noop_filter_returns_empty_vec() {
    let filter = NoopFilter;
    let result = filter
        .filter(Vec::new())
        .await
        .expect("NoopFilter.filter() should succeed");
    assert!(result.is_empty(), "NoopFilter must return an empty vec");
}

#[tokio::test]
async fn noop_analyzer_returns_empty_vec() {
    let analyzer = NoopAnalyzer;
    let result = analyzer
        .analyze(Vec::new())
        .await
        .expect("NoopAnalyzer.analyze() should succeed");
    assert!(result.is_empty(), "NoopAnalyzer must return an empty vec");
}

#[tokio::test]
async fn noop_crossref_returns_empty_vec() {
    let crossref = NoopCrossRef;
    let result = crossref
        .cross_reference(Vec::new())
        .await
        .expect("NoopCrossRef.cross_reference() should succeed");
    assert!(result.is_empty(), "NoopCrossRef must return an empty vec");
}

#[tokio::test]
async fn noop_reporter_succeeds() {
    let reporter = NoopReporter;
    reporter
        .report(&[])
        .await
        .expect("NoopReporter.report() should succeed");
}

// ---------------------------------------------------------------------------
// Enum dispatch tests (trait contract through the adapter enum)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn source_adapter_enum_dispatches_to_noop() {
    let adapter = SourceAdapter::Noop(NoopSource);
    let result = adapter.fetch().await.expect("SourceAdapter::Noop.fetch() should succeed");
    assert!(result.is_empty());
}

#[tokio::test]
async fn filter_adapter_enum_dispatches_to_noop() {
    let adapter = FilterAdapter::Noop(NoopFilter);
    let result = adapter
        .filter(Vec::new())
        .await
        .expect("FilterAdapter::Noop.filter() should succeed");
    assert!(result.is_empty());
}

#[tokio::test]
async fn analyzer_adapter_enum_dispatches_to_noop() {
    let adapter = AnalyzerAdapter::Noop(NoopAnalyzer);
    let result = adapter
        .analyze(Vec::new())
        .await
        .expect("AnalyzerAdapter::Noop.analyze() should succeed");
    assert!(result.is_empty());
}

#[tokio::test]
async fn crossref_adapter_enum_dispatches_to_noop() {
    let adapter = CrossRefAdapter::Noop(NoopCrossRef);
    let result = adapter
        .cross_reference(Vec::new())
        .await
        .expect("CrossRefAdapter::Noop.cross_reference() should succeed");
    assert!(result.is_empty());
}

#[tokio::test]
async fn reporter_adapter_enum_dispatches_to_noop() {
    let adapter = ReporterAdapter::Noop(NoopReporter);
    adapter
        .report(&[])
        .await
        .expect("ReporterAdapter::Noop.report() should succeed");
}

#[tokio::test]
async fn reporter_adapter_enum_dispatches_to_console() {
    let adapter = ReporterAdapter::Console(ConsoleReporter::new());
    adapter
        .report(&[])
        .await
        .expect("ReporterAdapter::Console.report(&[]) should succeed");
}

// ---------------------------------------------------------------------------
// Name contract test
// ---------------------------------------------------------------------------

#[test]
fn source_adapter_noop_name_returns_noop() {
    let source = NoopSource;
    assert_eq!(source.name(), "noop");

    let adapter = SourceAdapter::Noop(NoopSource);
    assert_eq!(adapter.name(), "noop");
}
