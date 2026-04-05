use askama::Template;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Json, Redirect, Response};

use crate::adapters::web::AppState;
use crate::adapters::web::templates::{DiffTableTemplate, DiffTemplate, ErrorTemplate};
use crate::domain::diff::compute_diff;

/// GET /diff — redirect to the latest two scans, or show an error.
pub async fn diff_default(State(state): State<AppState>) -> Response {
    let scans = match state.scan_store.list() {
        Ok(s) => s,
        Err(e) => {
            let tmpl = ErrorTemplate {
                status_code: 500,
                title: "Store Error".to_string(),
                message: format!("Failed to list scans: {e}"),
            };
            let html = tmpl
                .render()
                .unwrap_or_else(|_| "<h1>500 Internal Server Error</h1>".to_string());
            return (StatusCode::INTERNAL_SERVER_ERROR, Html(html)).into_response();
        }
    };

    if scans.len() < 2 {
        let tmpl = ErrorTemplate {
            status_code: 404,
            title: "Not Enough Scans".to_string(),
            message: "At least two scans are required to produce a diff.".to_string(),
        };
        let html = tmpl
            .render()
            .unwrap_or_else(|_| "<h1>404 Not Found</h1>".to_string());
        return (StatusCode::NOT_FOUND, Html(html)).into_response();
    }

    // Redirect to diff of second-latest (A) vs latest (B)
    let id_b = &scans[0].id;
    let id_a = &scans[1].id;
    Redirect::to(&format!("/diff/{id_a}/{id_b}")).into_response()
}

/// GET /diff/{id_a}/{id_b} — render diff page (full or HTMX partial).
pub async fn diff_html(
    State(state): State<AppState>,
    Path((id_a, id_b)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let (meta_a, results_a) = match load_scan(&state, &id_a) {
        Ok(pair) => pair,
        Err(resp) => return resp,
    };
    let (meta_b, results_b) = match load_scan(&state, &id_b) {
        Ok(pair) => pair,
        Err(resp) => return resp,
    };

    let diff = compute_diff(meta_a, meta_b, &results_a, &results_b);

    // If HTMX request, return the swappable partial only
    if headers.contains_key("HX-Request") {
        let tmpl = DiffTableTemplate { diff };
        return Html(
            tmpl.render()
                .unwrap_or_else(|e| format!("<p>Template error: {e}</p>")),
        )
        .into_response();
    }

    let tmpl = DiffTemplate { diff };
    Html(
        tmpl.render()
            .unwrap_or_else(|e| format!("<h1>Template error</h1><p>{e}</p>")),
    )
    .into_response()
}

/// GET /api/diff/{id_a}/{id_b} — return diff as JSON.
pub async fn diff_api(
    State(state): State<AppState>,
    Path((id_a, id_b)): Path<(String, String)>,
) -> Response {
    let (meta_a, results_a) = match load_scan(&state, &id_a) {
        Ok(pair) => pair,
        Err(resp) => return resp,
    };
    let (meta_b, results_b) = match load_scan(&state, &id_b) {
        Ok(pair) => pair,
        Err(resp) => return resp,
    };

    let diff = compute_diff(meta_a, meta_b, &results_a, &results_b);
    Json(diff).into_response()
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn load_scan(
    state: &AppState,
    id: &str,
) -> Result<
    (
        crate::infra::scan_store::ScanMeta,
        Vec<crate::domain::model::CrossRefResult>,
    ),
    Response,
> {
    // Get scan metadata by listing all scans and finding the matching ID
    let scans = state.scan_store.list().map_err(|e| {
        let tmpl = ErrorTemplate {
            status_code: 500,
            title: "Store Error".to_string(),
            message: format!("Failed to list scans: {e}"),
        };
        let html = tmpl
            .render()
            .unwrap_or_else(|_| "<h1>500 Internal Server Error</h1>".to_string());
        (StatusCode::INTERNAL_SERVER_ERROR, Html(html)).into_response()
    })?;

    let meta = scans
        .into_iter()
        .find(|s| s.id == id)
        .ok_or_else(|| {
            let tmpl = ErrorTemplate {
                status_code: 404,
                title: "Scan Not Found".to_string(),
                message: format!("Scan '{id}' does not exist."),
            };
            let html = tmpl
                .render()
                .unwrap_or_else(|_| "<h1>404 Not Found</h1>".to_string());
            (StatusCode::NOT_FOUND, Html(html)).into_response()
        })?;

    let results = state.scan_store.load(id).map_err(|e| {
        let tmpl = ErrorTemplate {
            status_code: 500,
            title: "Load Error".to_string(),
            message: format!("Failed to load scan '{id}': {e}"),
        };
        let html = tmpl
            .render()
            .unwrap_or_else(|_| "<h1>500 Internal Server Error</h1>".to_string());
        (StatusCode::INTERNAL_SERVER_ERROR, Html(html)).into_response()
    })?;

    Ok((meta, results))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::web::router;
    use crate::adapters::web::state::ScanStatus;
    use crate::config::AppConfig;
    use crate::domain::model::{AnalysisResult, CrossRefResult, FeedEntry, RepoCandidate, RepoMatch};
    use axum::body::Body;
    use axum::http::Request;
    use chrono::Utc;
    use http_body_util::BodyExt;
    use std::sync::Arc;
    use tokio::sync::{Mutex, RwLock, broadcast};
    use tower::ServiceExt;
    use url::Url;

    fn make_result(repo: &str, relevance: f64) -> CrossRefResult {
        CrossRefResult {
            analysis: AnalysisResult {
                candidate: RepoCandidate {
                    entry: FeedEntry {
                        title: repo.to_string(),
                        repo_url: Url::parse(&format!("https://github.com/owner/{repo}")).unwrap(),
                        description: None,
                        published: Some(Utc::now()),
                        source_name: "test".to_string(),
                    },
                    stars: 50,
                    language: Some("Rust".to_string()),
                    topics: vec![],
                    fork: false,
                    archived: false,
                    owner: "owner".to_string(),
                    repo_name: repo.to_string(),
                    category: Default::default(),
                    semantic_score: 0.0,
                    pushed_at: None,
                },
                summary: "summary".to_string(),
                key_features: vec![],
                tech_stack: vec![],
                relevance_score: relevance,
            },
            matched_repos: vec![RepoMatch {
                own_repo: "my/repo".to_string(),
                relevance: 0.5,
                reason: "test".to_string(),
            }],
            ideas: vec![],
            overall_relevance: relevance,
        }
    }

    /// Returns `(AppState, id_a, id_b, _dir)`.
    /// The caller MUST bind `_dir` to keep the temp directory alive for the
    /// duration of the test. Dropping it early deletes the scan files on disk
    /// and causes 404 responses.
    fn test_state_with_two_scans() -> (AppState, String, String, tempfile::TempDir) {
        let (progress_tx, _) = broadcast::channel(16);
        let dir = tempfile::tempdir().unwrap();
        let store = crate::infra::scan_store::ScanResultStore::new(dir.path().join("results"));

        let meta_a = store.save(&[make_result("repo-a", 0.5)]).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        let meta_b = store.save(&[make_result("repo-a", 0.7), make_result("repo-b", 0.8)]).unwrap();

        let state = AppState {
            config: AppConfig::default(),
            scan_status: Arc::new(Mutex::new(ScanStatus::default())),
            last_results: Arc::new(RwLock::new(None)),
            progress_tx,
            scan_store: Arc::new(store),
        };

        (state, meta_a.id, meta_b.id, dir)
    }

    #[tokio::test]
    async fn diff_html_returns_200_for_valid_ids() {
        let (state, id_a, id_b, _dir) = test_state_with_two_scans();
        let app = router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/diff/{id_a}/{id_b}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), 200);
        let body = BodyExt::collect(response.into_body())
            .await
            .unwrap()
            .to_bytes();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(text.contains("<!DOCTYPE html>"), "should render full page");
        assert!(text.contains("repo-b"), "should show new repo");
    }

    #[tokio::test]
    async fn diff_api_returns_json_with_correct_shape() {
        let (state, id_a, id_b, _dir) = test_state_with_two_scans();
        let app = router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/diff/{id_a}/{id_b}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), 200);
        let body = BodyExt::collect(response.into_body())
            .await
            .unwrap()
            .to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert!(json["new_repos"].is_array(), "should have new_repos array");
        assert!(json["removed_repos"].is_array(), "should have removed_repos array");
        assert!(json["changed_repos"].is_array(), "should have changed_repos array");

        // repo-b is new in B
        let new_repos = json["new_repos"].as_array().unwrap();
        assert_eq!(new_repos.len(), 1);
        assert_eq!(
            new_repos[0]["analysis"]["candidate"]["repo_name"].as_str().unwrap(),
            "repo-b"
        );

        // changed_repos should have correct score_delta sign
        let changed = json["changed_repos"].as_array().unwrap();
        if !changed.is_empty() {
            let delta = changed[0]["score_delta"].as_f64().unwrap();
            assert!(delta > 0.0, "score_delta should be positive when B > A");
        }
    }

    #[tokio::test]
    async fn diff_html_returns_404_for_invalid_scan_id() {
        let (state, id_a, _id_b, _dir) = test_state_with_two_scans();
        let app = router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/diff/{id_a}/nonexistent-scan-id"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), 404);
    }

    #[tokio::test]
    async fn diff_default_redirects_or_renders_latest_two_scans() {
        let (state, _id_a, _id_b, _dir) = test_state_with_two_scans();
        let app = router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/diff")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Either a redirect (3xx) or a 200 are acceptable
        let status = response.status().as_u16();
        assert!(
            (300..=399).contains(&status) || status == 200,
            "expected redirect or 200, got {status}"
        );
    }
}
