use repo_radar::adapters::analyzer::NoopAnalyzer;
use repo_radar::adapters::crossref::NoopCrossRef;
use repo_radar::adapters::filter::NoopFilter;
use repo_radar::adapters::reporter::NoopReporter;
use repo_radar::adapters::source::NoopSource;
use repo_radar::infra::seen::SeenStore;
use repo_radar::pipeline::Pipeline;

#[tokio::test]
async fn pipeline_with_noop_adapters_runs_successfully() {
    let dir = tempfile::tempdir().unwrap();
    let seen_path = dir.path().join("seen.json");
    let seen = SeenStore::load(&seen_path).unwrap();

    let mut pipeline = Pipeline::new(
        NoopSource,
        NoopFilter,
        NoopAnalyzer,
        NoopCrossRef,
        NoopReporter,
        seen,
        None,
    );

    let report = pipeline.run().await.unwrap();

    assert_eq!(report.entries_fetched, 0);
    assert_eq!(report.entries_new, 0);
    assert_eq!(report.candidates_filtered, 0);
    assert_eq!(report.analyzed, 0);
    assert_eq!(report.crossrefed, 0);
    assert_eq!(report.reported, 0);
}

#[tokio::test]
async fn pipeline_report_display_format() {
    let dir = tempfile::tempdir().unwrap();
    let seen_path = dir.path().join("seen.json");
    let seen = SeenStore::load(&seen_path).unwrap();

    let mut pipeline = Pipeline::new(
        NoopSource,
        NoopFilter,
        NoopAnalyzer,
        NoopCrossRef,
        NoopReporter,
        seen,
        None,
    );

    let report = pipeline.run().await.unwrap();
    let display = report.to_string();

    assert!(display.contains("Pipeline complete"));
    assert!(display.contains("0 fetched"));
    assert!(display.contains("0 new"));
    assert!(display.contains("0 filtered"));
    assert!(display.contains("0 analyzed"));
    assert!(display.contains("0 cross-referenced"));
    assert!(display.contains("0 reported"));
}

#[tokio::test]
async fn pipeline_persists_seen_store() {
    let dir = tempfile::tempdir().unwrap();
    let seen_path = dir.path().join("seen.json");
    let seen = SeenStore::load(&seen_path).unwrap();

    let mut pipeline = Pipeline::new(
        NoopSource,
        NoopFilter,
        NoopAnalyzer,
        NoopCrossRef,
        NoopReporter,
        seen,
        None,
    );

    pipeline.run().await.unwrap();

    // Even with no entries, the seen store file should be written
    assert!(seen_path.exists());
}
