pub mod assets;
pub mod auth;
pub mod error;
pub mod handlers;
pub mod state;
pub mod templates;

use std::sync::Arc;

use askama::Template;
use axum::{Extension, Router, middleware};
use axum::extract::Request;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, post};
use serde_json::json;
use tokio::sync::{Mutex, RwLock, broadcast};

use self::auth::ExpectedToken;

use crate::config::AppConfig;
use crate::domain::model::CrossRefResult;

use self::state::{ScanProgress, ScanStatus};

/// Shared application state for the web server.
#[derive(Clone)]
pub struct AppState {
    pub config: AppConfig,
    pub scan_status: Arc<Mutex<ScanStatus>>,
    pub last_results: Arc<RwLock<Option<Vec<CrossRefResult>>>>,
    pub progress_tx: broadcast::Sender<ScanProgress>,
}

/// Fallback handler for unmatched routes.
/// Returns JSON for /api/* paths, HTML error page for everything else.
async fn fallback_handler(req: Request) -> Response {
    let path = req.uri().path().to_string();
    if path.starts_with("/api/") {
        (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": "The requested API endpoint does not exist" })),
        )
            .into_response()
    } else {
        let tmpl = templates::ErrorTemplate {
            status_code: 404,
            title: "Not Found".to_string(),
            message: "The page you are looking for does not exist.".to_string(),
        };
        let html = tmpl
            .render()
            .unwrap_or_else(|_| "<h1>404 Not Found</h1>".to_string());
        (StatusCode::NOT_FOUND, axum::response::Html(html)).into_response()
    }
}

/// Build the web router with all routes mounted.
///
/// When `AppConfig.general.dashboard_token` is set, all routes require a valid
/// `Authorization: Bearer <token>` header. When unset, the dashboard is open.
pub fn router(state: AppState) -> Router {
    let dashboard_token = state.config.general.dashboard_token.clone();

    let app = Router::new()
        .route("/", get(handlers::dashboard::index))
        .route("/config", get(handlers::pages::config_page))
        .route("/api/config", get(handlers::pages::config_json))
        .route("/api/results", get(handlers::api::get_results))
        .route("/api/scan", post(handlers::scan::start_scan))
        .route("/api/scan/events", get(handlers::scan::scan_events))
        .route("/reports", get(handlers::pages::reports_page))
        .route("/reports/{id}", get(handlers::pages::report_detail))
        .route("/compare/{owner}/{repo}", get(handlers::compare::compare_view))
        .route("/static/{*path}", get(assets::serve_static))
        .fallback(fallback_handler)
        .with_state(state);

    if let Some(token) = dashboard_token {
        tracing::info!("Dashboard auth enabled via REPO_RADAR_DASHBOARD_TOKEN");
        app.layer(middleware::from_fn(auth::require_bearer_token))
            .layer(Extension(ExpectedToken(token)))
    } else {
        tracing::warn!(
            "Dashboard running without auth \u{2014} only accessible on localhost"
        );
        app
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    fn test_state() -> AppState {
        let (progress_tx, _) = broadcast::channel(16);
        AppState {
            config: AppConfig::default(),
            scan_status: Arc::new(Mutex::new(ScanStatus::default())),
            last_results: Arc::new(RwLock::new(None)),
            progress_tx,
        }
    }

    #[test]
    fn base_template_renders_html5() {
        use askama::Template;

        #[derive(Template)]
        #[template(
            source = r#"{% extends "base.html" %}{% block content %}<p>hello</p>{% endblock %}"#,
            ext = "html"
        )]
        struct TestPage;

        let rendered = TestPage.render().unwrap();
        assert!(rendered.contains("<!DOCTYPE html>"), "should have HTML5 doctype");
        assert!(rendered.contains("repo-radar"), "should have brand name");
        assert!(rendered.contains("cdn.tailwindcss.com"), "should load Tailwind");
        assert!(rendered.contains("htmx.org@2.0.4"), "should load HTMX");
        assert!(rendered.contains("chart.js"), "should load Chart.js");
        assert!(rendered.contains("<p>hello</p>"), "should render block content");
    }

    #[tokio::test]
    async fn static_asset_route_serves_css() {
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

        assert_eq!(response.status(), 200);
        let ct = response
            .headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        assert!(ct.contains("css"), "content-type should be CSS, got {ct}");
    }

    #[tokio::test]
    async fn static_asset_route_returns_404_for_missing() {
        let app = router(test_state());

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/static/nope.txt")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), 404);
    }

    #[tokio::test]
    async fn index_returns_dashboard_html() {
        let app = router(test_state());

        let response = app
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), 200);

        let body = BodyExt::collect(response.into_body())
            .await
            .unwrap()
            .to_bytes();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(text.contains("<!DOCTYPE html>"), "should render full HTML page");
        assert!(text.contains("repo-radar"), "should contain brand name");
        assert!(
            text.contains("No scan results yet"),
            "should show empty state when no results"
        );
    }
}
