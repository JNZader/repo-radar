pub mod assets;
pub mod error;
pub mod handlers;
pub mod state;
pub mod templates;

use std::sync::Arc;

use axum::Router;
use axum::routing::get;
use tokio::sync::{Mutex, RwLock, broadcast};

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

/// Build the web router with all routes mounted.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(handlers::dashboard::index))
        .route("/api/results", get(handlers::api::get_results))
        .route("/static/{*path}", get(assets::serve_static))
        .with_state(state)
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
