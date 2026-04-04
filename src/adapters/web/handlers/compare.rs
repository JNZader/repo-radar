use askama::Template;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};

use crate::adapters::web::AppState;
use crate::adapters::web::templates::{
    build_compare_data, CompareTemplate, ErrorTemplate,
};

/// GET /compare/{owner}/{repo} — render the comparison view for a discovered repo.
pub async fn compare_view(
    State(state): State<AppState>,
    Path((owner, repo)): Path<(String, String)>,
) -> Response {
    let guard = state.last_results.read().await;
    let results = match guard.as_ref() {
        Some(r) => r.clone(),
        None => {
            drop(guard);
            let tmpl = ErrorTemplate {
                status_code: 404,
                title: "No Scan Results".to_string(),
                message: "No scan results available. Run a scan first.".to_string(),
            };
            let html = tmpl
                .render()
                .unwrap_or_else(|_| "<h1>404 Not Found</h1>".to_string());
            return (StatusCode::NOT_FOUND, Html(html)).into_response();
        }
    };
    drop(guard);

    // Find the CrossRefResult matching owner/repo
    let result = results.iter().find(|r| {
        r.analysis.candidate.owner == owner && r.analysis.candidate.repo_name == repo
    });

    let Some(result) = result else {
        let tmpl = ErrorTemplate {
            status_code: 404,
            title: "Repository Not Found".to_string(),
            message: format!(
                "Repository {owner}/{repo} was not found in the current scan results."
            ),
        };
        let html = tmpl
            .render()
            .unwrap_or_else(|_| "<h1>404 Not Found</h1>".to_string());
        return (StatusCode::NOT_FOUND, Html(html)).into_response();
    };

    let (discovered, matches, unique_topics) = build_compare_data(result);

    let tmpl = CompareTemplate {
        discovered,
        matches,
        unique_topics,
    };

    Html(tmpl.render().unwrap_or_else(|e| {
        format!("<h1>Template error</h1><p>{e}</p>")
    }))
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::web::router;
    use crate::adapters::web::state::ScanStatus;
    use crate::config::AppConfig;
    use crate::domain::model::{
        AnalysisResult, CrossRefResult, FeedEntry, RepoCandidate, RepoMatch,
    };
    use axum::body::Body;
    use axum::http::Request;
    use chrono::Utc;
    use http_body_util::BodyExt;
    use std::sync::Arc;
    use tokio::sync::{Mutex, RwLock, broadcast};
    use tower::ServiceExt;
    use url::Url;

    fn mock_result_with_matches() -> CrossRefResult {
        CrossRefResult {
            analysis: AnalysisResult {
                candidate: RepoCandidate {
                    entry: FeedEntry {
                        title: "cool-tool".to_string(),
                        repo_url: Url::parse("https://github.com/testowner/cool-tool").unwrap(),
                        description: Some("A cool CLI tool".into()),
                        published: Some(Utc::now()),
                        source_name: "GitHub Trending".into(),
                    },
                    stars: 500,
                    language: Some("Rust".to_string()),
                    topics: vec!["cli".into(), "async".into(), "tooling".into()],
                    fork: false,
                    archived: false,
                    owner: "testowner".into(),
                    repo_name: "cool-tool".into(),
                    category: Default::default(),
                },
                summary: "A great CLI tool for async workflows".into(),
                key_features: vec!["fast".into(), "safe".into()],
                tech_stack: vec!["Rust".into(), "tokio".into()],
                relevance_score: 0.85,
            },
            matched_repos: vec![
                RepoMatch {
                    own_repo: "myuser/my-cli".into(),
                    relevance: 0.67,
                    reason: "shared language: Rust; shared topics: cli".into(),
                },
                RepoMatch {
                    own_repo: "myuser/async-lib".into(),
                    relevance: 0.5,
                    reason: "shared language: Rust; shared topics: async".into(),
                },
            ],
            ideas: vec!["Explore patterns from cool-tool".into()],
            overall_relevance: 0.585,
        }
    }

    fn test_state_with_results(results: Vec<CrossRefResult>) -> AppState {
        let (progress_tx, _) = broadcast::channel(16);
        let dir = tempfile::tempdir().unwrap();
        AppState {
            config: AppConfig::default(),
            scan_status: Arc::new(Mutex::new(ScanStatus::default())),
            last_results: Arc::new(RwLock::new(Some(results))),
            progress_tx,
            scan_store: Arc::new(crate::infra::scan_store::ScanResultStore::new(
                dir.path().join("results"),
            )),
        }
    }

    fn test_state_empty() -> AppState {
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
    async fn compare_view_renders_comparison() {
        let results = vec![mock_result_with_matches()];
        let app = router(test_state_with_results(results));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/compare/testowner/cool-tool")
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
        assert!(text.contains("cool-tool"), "should show repo name");
        assert!(text.contains("testowner"), "should show owner");
        assert!(text.contains("500"), "should show stars");
        assert!(text.contains("Rust"), "should show language");
        assert!(text.contains("my-cli"), "should show matched repo");
        assert!(text.contains("async-lib"), "should show second match");
    }

    #[tokio::test]
    async fn compare_view_404_when_repo_not_found() {
        let results = vec![mock_result_with_matches()];
        let app = router(test_state_with_results(results));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/compare/unknown/nonexistent")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), 404);
        let body = BodyExt::collect(response.into_body())
            .await
            .unwrap()
            .to_bytes();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(text.contains("not found"), "should show not found message");
    }

    #[tokio::test]
    async fn compare_view_404_when_no_scan_results() {
        let app = router(test_state_empty());

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/compare/owner/repo")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), 404);
        let body = BodyExt::collect(response.into_body())
            .await
            .unwrap()
            .to_bytes();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(text.contains("No scan results"), "should indicate no scan results");
    }
}
