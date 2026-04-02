use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tokio::sync::{Mutex, RwLock, broadcast};
use tower::ServiceExt;

use repo_radar::adapters::web::state::ScanStatus;
use repo_radar::adapters::web::{AppState, router};
use repo_radar::config::AppConfig;
use repo_radar::domain::model::{
    AnalysisResult, CrossRefResult, FeedEntry, RepoCandidate, RepoMatch,
};

fn test_state() -> AppState {
    let (progress_tx, _) = broadcast::channel(16);
    AppState {
        config: AppConfig::default(),
        scan_status: Arc::new(Mutex::new(ScanStatus::default())),
        last_results: Arc::new(RwLock::new(None)),
        progress_tx,
    }
}

fn test_state_with_results(results: Vec<CrossRefResult>) -> AppState {
    let (progress_tx, _) = broadcast::channel(16);
    AppState {
        config: AppConfig::default(),
        scan_status: Arc::new(Mutex::new(ScanStatus::default())),
        last_results: Arc::new(RwLock::new(Some(results))),
        progress_tx,
    }
}

fn mock_result(name: &str, stars: u64, lang: &str, relevance: f64) -> CrossRefResult {
    use chrono::Utc;
    use url::Url;

    CrossRefResult {
        analysis: AnalysisResult {
            candidate: RepoCandidate {
                entry: FeedEntry {
                    title: name.to_string(),
                    repo_url: Url::parse(&format!("https://github.com/owner/{name}")).unwrap(),
                    description: Some("A test repo".into()),
                    published: Some(Utc::now()),
                    source_name: "GitHub Trending".into(),
                },
                stars,
                language: Some(lang.to_string()),
                topics: vec!["cli".into()],
                fork: false,
                archived: false,
                owner: "owner".into(),
                repo_name: name.to_string(),
            },
            summary: "Test summary".into(),
            key_features: vec!["fast".into()],
            tech_stack: vec![lang.to_string()],
            relevance_score: relevance,
        },
        matched_repos: vec![RepoMatch {
            own_repo: "my-project".into(),
            relevance: 0.75,
            reason: "Similar tech".into(),
        }],
        ideas: vec!["Use this pattern".into()],
        overall_relevance: relevance,
    }
}

async fn get_body_text(response: axum::http::Response<Body>) -> String {
    let body = BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    String::from_utf8(body.to_vec()).unwrap()
}

// ── GET / ───────────────────────────────────────────────────────────

#[tokio::test]
async fn get_index_returns_200_with_html() {
    let app = router(test_state());
    let response = app
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
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
    assert!(ct.contains("html"), "expected HTML content-type, got {ct}");
}

// ── GET /config ─────────────────────────────────────────────────────

#[tokio::test]
async fn get_config_returns_200_with_html() {
    let app = router(test_state());
    let response = app
        .oneshot(
            Request::builder()
                .uri("/config")
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
    assert!(ct.contains("html"), "expected HTML content-type, got {ct}");
}

// ── GET /api/config ─────────────────────────────────────────────────

#[tokio::test]
async fn get_api_config_returns_200_with_json() {
    let app = router(test_state());
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/config")
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
    assert!(ct.contains("json"), "expected JSON content-type, got {ct}");
}

// ── GET /reports ────────────────────────────────────────────────────

#[tokio::test]
async fn get_reports_returns_200_with_html() {
    let app = router(test_state());
    let response = app
        .oneshot(
            Request::builder()
                .uri("/reports")
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
    assert!(ct.contains("html"), "expected HTML content-type, got {ct}");
}

// ── GET /reports/nonexistent ────────────────────────────────────────

#[tokio::test]
async fn get_reports_nonexistent_returns_404() {
    let app = router(test_state());
    let response = app
        .oneshot(
            Request::builder()
                .uri("/reports/nonexistent")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// ── GET /api/results ────────────────────────────────────────────────

#[tokio::test]
async fn get_api_results_returns_200_with_json() {
    let app = router(test_state());
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/results")
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
    assert!(ct.contains("json"), "expected JSON content-type, got {ct}");
}

// ── POST /api/scan ──────────────────────────────────────────────────

#[tokio::test]
async fn post_api_scan_returns_202_when_idle() {
    let app = router(test_state());
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
}

#[tokio::test]
async fn post_api_scan_returns_409_when_busy() {
    let state = test_state();
    {
        let mut status = state.scan_status.lock().await;
        *status = ScanStatus::Running {
            started_at: chrono::Utc::now(),
        };
    }

    let app = router(state);
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
}

// ── GET /static/css/app.css ─────────────────────────────────────────

#[tokio::test]
async fn get_static_css_returns_200_with_css_content_type() {
    let app = router(test_state());
    let response = app
        .oneshot(
            Request::builder()
                .uri("/static/css/app.css")
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
    assert!(ct.contains("css"), "expected CSS content-type, got {ct}");
}

// ── GET /nonexistent → 404 HTML ─────────────────────────────────────

#[tokio::test]
async fn get_nonexistent_returns_404_html() {
    let app = router(test_state());
    let response = app
        .oneshot(
            Request::builder()
                .uri("/nonexistent")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let ct = response
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(ct.contains("html"), "expected HTML error page, got {ct}");

    let text = get_body_text(response).await;
    assert!(text.contains("404"), "error page should contain status code");
    assert!(text.contains("Not Found"), "error page should contain title");
}

// ── GET /api/nonexistent → 404 JSON ────────────────────────────────

#[tokio::test]
async fn get_api_nonexistent_returns_404_json() {
    let app = router(test_state());
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/nonexistent")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let ct = response
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(ct.contains("json"), "expected JSON error, got {ct}");

    let text = get_body_text(response).await;
    assert!(text.contains("not_found"), "JSON error should contain error field");
}

// ── Dashboard with results includes chart data ──────────────────────

#[tokio::test]
async fn dashboard_with_results_includes_chart_data() {
    let results = vec![
        mock_result("alpha", 100, "Rust", 0.85),
        mock_result("beta", 50, "Go", 0.45),
    ];
    let app = router(test_state_with_results(results));
    let response = app
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let text = get_body_text(response).await;
    assert!(
        text.contains("relevance-chart"),
        "should have relevance chart canvas"
    );
    assert!(
        text.contains("language-chart"),
        "should have language chart canvas"
    );
    assert!(
        text.contains("chart-data"),
        "should have chart data script tag"
    );
}
