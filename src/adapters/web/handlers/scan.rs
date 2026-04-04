use std::convert::Infallible;
use std::pin::Pin;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Json, Response};
use chrono::Utc;
use futures_util::stream::{self, Stream};
use serde_json::json;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;
use tracing::{error, info};

use crate::adapters::analyzer::{AnalyzerAdapter, NoopAnalyzer, RepoforgeAnalyzer};
use crate::adapters::categorizer::{CategorizerAdapter, KeywordCategorizer};
use crate::adapters::crossref::github_crossref::GitHubCrossRef;
use crate::adapters::crossref::{CrossRefAdapter, NoopCrossRef};
use crate::adapters::filter::{FilterAdapter, GitHubMetadataFilter, NoopFilter};
use crate::adapters::reporter::{NoopReporter, ReporterAdapter};
use crate::adapters::source::{NoopSource, RssSource, SourceAdapter};
use crate::adapters::web::state::ScanStatus;
use crate::adapters::web::AppState;
use crate::config::AppConfig;
use crate::infra::cache::RepoCache;
use crate::infra::seen::SeenStore;
use crate::pipeline::Pipeline;

/// POST /api/scan — trigger a new scan.
///
/// Returns 409 if a scan is already running, 202 if started successfully.
pub async fn start_scan(State(state): State<AppState>) -> Response {
    // Check if already running
    {
        let status = state.scan_status.lock().await;
        if matches!(*status, ScanStatus::Running { .. }) {
            return (
                StatusCode::CONFLICT,
                Json(json!({ "error": "scan already running" })),
            )
                .into_response();
        }
    }

    // Set status to Running
    {
        let mut status = state.scan_status.lock().await;
        *status = ScanStatus::Running {
            started_at: Utc::now(),
        };
    }

    let config = state.config.clone();
    let progress_tx = state.progress_tx.clone();
    let scan_status = state.scan_status.clone();
    let last_results = state.last_results.clone();
    let scan_store = state.scan_store.clone();

    // Spawn background task
    tokio::spawn(async move {
        let result = run_pipeline(&config, progress_tx.clone()).await;

        match result {
            Ok((report, crossref_results)) => {
                info!(%report, "scan completed successfully");

                // Persist results to disk
                if let Err(e) = scan_store.save(&crossref_results) {
                    error!(%e, "failed to persist scan results to disk");
                }

                // Store results in memory for the current session
                {
                    let mut results = last_results.write().await;
                    *results = Some(crossref_results);
                }

                // Set status to Complete
                {
                    let mut status = scan_status.lock().await;
                    *status = ScanStatus::Complete {
                        finished_at: Utc::now(),
                        report,
                    };
                }
            }
            Err(err) => {
                error!(%err, "scan failed");

                // Emit error event
                let _ = progress_tx.send(crate::pipeline::ScanProgress {
                    stage: "error".into(),
                    percent: 0,
                    message: format!("Scan failed: {err}"),
                });

                // Reset status to Idle
                {
                    let mut status = scan_status.lock().await;
                    *status = ScanStatus::Idle;
                }
            }
        }
    });

    (StatusCode::ACCEPTED, Json(json!({ "status": "started" }))).into_response()
}

/// Build and run the pipeline, returning the report and cross-ref results.
async fn run_pipeline(
    config: &AppConfig,
    progress_tx: tokio::sync::broadcast::Sender<crate::pipeline::ScanProgress>,
) -> Result<
    (
        crate::pipeline::PipelineReport,
        Vec<crate::domain::model::CrossRefResult>,
    ),
    crate::infra::error::PipelineError,
> {
    let seen_path = config.general.data_dir.join("seen.json");
    let seen = SeenStore::load(&seen_path)?;

    // Source: RSS if feeds configured, otherwise Noop
    let source = if config.feeds.is_empty() {
        SourceAdapter::Noop(NoopSource)
    } else {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| {
                crate::infra::error::PipelineError::Config(format!("building HTTP client: {e}"))
            })?;
        SourceAdapter::Rss(RssSource::new(config.feeds.clone(), client))
    };

    // Filter: GitHub metadata if token available, otherwise Noop
    let filter = if config.general.github_token.is_some() || !config.feeds.is_empty() {
        let cache_dir = config
            .cache
            .cache_dir
            .clone()
            .unwrap_or_else(|| config.general.data_dir.join("cache"));
        let cache_path = cache_dir.join("repo_metadata.json");
        let cache_ttl = std::time::Duration::from_secs(config.cache.ttl_secs);
        let repo_cache = RepoCache::load(&cache_path, cache_ttl)
            .map_err(|e| crate::infra::error::PipelineError::Config(format!("cache: {e}")))?;

        let gh_filter = GitHubMetadataFilter::new(
            config.filter.clone(),
            config.general.github_token.as_deref(),
            Some(repo_cache),
            config.cache.rate_limit_threshold,
        )
        .map_err(|e| crate::infra::error::PipelineError::Config(format!("filter: {e}")))?;
        FilterAdapter::GitHubMetadata(Box::new(gh_filter))
    } else {
        FilterAdapter::Noop(NoopFilter)
    };

    // Analyzer: RepoForge if path configured, otherwise Noop
    let analyzer = if let Some(ref repoforge_path) = config.analyzer.repoforge_path {
        AnalyzerAdapter::Repoforge(RepoforgeAnalyzer::new(
            repoforge_path.clone(),
            config.analyzer.timeout_secs,
        ))
    } else {
        AnalyzerAdapter::Noop(NoopAnalyzer)
    };

    // CrossRef: GitHub if username configured, otherwise Noop
    let crossref = if let Some(ref username) = config.crossref.github_username {
        let gh_crossref =
            GitHubCrossRef::new(username.clone(), config.general.github_token.as_deref())
                .map_err(|e| {
                    crate::infra::error::PipelineError::Config(format!("crossref: {e}"))
                })?;
        CrossRefAdapter::GitHub(Box::new(gh_crossref))
    } else {
        CrossRefAdapter::Noop(NoopCrossRef)
    };

    // Categorizer: always use keyword-based categorizer
    let categorizer = CategorizerAdapter::Keyword(KeywordCategorizer::new());

    // For the web scan, use NoopReporter — results are stored in state
    let reporter = ReporterAdapter::Noop(NoopReporter);

    let mut pipeline = Pipeline::new(
        source,
        filter,
        categorizer,
        analyzer,
        crossref,
        reporter,
        seen,
        Some(progress_tx),
    );

    let (report, crossref_results) = pipeline.run().await?;

    Ok((report, crossref_results))
}

type SseStream = Pin<Box<dyn Stream<Item = Result<Event, Infallible>> + Send>>;

/// GET /api/scan/events — SSE endpoint for scan progress.
pub async fn scan_events(
    State(state): State<AppState>,
) -> Sse<axum::response::sse::KeepAliveStream<SseStream>> {
    let status = state.scan_status.lock().await;
    let is_running = matches!(*status, ScanStatus::Running { .. });
    drop(status);

    let event_stream: SseStream = if !is_running {
        // Not running: send a single idle event and close
        Box::pin(stream::once(async {
            Ok::<_, Infallible>(Event::default().event("idle").data("no scan running"))
        }))
    } else {
        let rx = state.progress_tx.subscribe();
        Box::pin(
            BroadcastStream::new(rx).filter_map(|result| match result {
                Ok(progress) => {
                    let event_type = match progress.stage.as_str() {
                        "complete" => "complete",
                        "error" => "error",
                        _ => "progress",
                    };
                    let data = serde_json::to_string(&progress).unwrap_or_default();
                    Some(Ok::<_, Infallible>(
                        Event::default().event(event_type).data(data),
                    ))
                }
                Err(_) => None, // Lagged or closed — skip
            }),
        )
    };

    Sse::new(event_stream).keep_alive(KeepAlive::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::web::{self, AppState};
    use crate::config::AppConfig;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use std::sync::Arc;
    use tokio::sync::{Mutex, RwLock, broadcast};
    use tower::ServiceExt;

    fn test_state() -> AppState {
        let (progress_tx, _) = broadcast::channel(16);
        let dir = tempfile::tempdir().unwrap();
        AppState {
            config: AppConfig::default(),
            scan_status: Arc::new(Mutex::new(ScanStatus::default())),
            last_results: Arc::new(RwLock::new(None)),
            progress_tx,
            scan_store: Arc::new(crate::infra::scan_store::ScanResultStore::new(
                dir.path().join("results"),
            )),
        }
    }

    #[tokio::test]
    async fn post_scan_returns_202_when_idle() {
        let app = web::router(test_state());

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/scan")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::ACCEPTED);
        let body = BodyExt::collect(response.into_body())
            .await
            .unwrap()
            .to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "started");
    }

    #[tokio::test]
    async fn post_scan_returns_409_when_running() {
        let state = test_state();
        {
            let mut status = state.scan_status.lock().await;
            *status = ScanStatus::Running {
                started_at: Utc::now(),
            };
        }

        let app = web::router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/scan")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body = BodyExt::collect(response.into_body())
            .await
            .unwrap()
            .to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"], "scan already running");
    }

    #[tokio::test]
    async fn sse_events_returns_idle_when_not_running() {
        let app = web::router(test_state());

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/scan/events")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let ct = response
            .headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        assert!(
            ct.contains("text/event-stream"),
            "should be SSE content type, got {ct}"
        );
    }
}
