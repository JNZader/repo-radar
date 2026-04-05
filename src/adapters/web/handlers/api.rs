use askama::Template;
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::{Html, IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::adapters::web::AppState;
use crate::adapters::web::templates::ResultsTableTemplate;
use crate::domain::model::CrossRefResult;

/// Query parameters for the results API endpoint.
#[derive(Debug, Deserialize)]
pub struct ResultsQuery {
    #[serde(default = "default_sort")]
    pub sort: String,
    #[serde(default = "default_dir")]
    pub dir: String,
    #[serde(default)]
    pub lang: String,
    #[serde(default)]
    pub topic: String,
    #[serde(default = "default_page")]
    pub page: usize,
    #[serde(default = "default_page_size")]
    pub page_size: usize,
}

fn default_sort() -> String {
    "stars".into()
}
fn default_dir() -> String {
    "desc".into()
}
fn default_page() -> usize {
    1
}
fn default_page_size() -> usize {
    20
}

/// JSON response wrapper for API clients.
#[derive(Serialize, Deserialize)]
pub struct ResultsResponse {
    pub results: Vec<CrossRefResult>,
    pub page: usize,
    pub page_size: usize,
    pub total_pages: usize,
    pub total_results: usize,
}

/// GET /api/results — returns sorted, filtered, paginated results.
/// If HX-Request header is present, returns HTML partial; otherwise JSON.
pub async fn get_results(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<ResultsQuery>,
) -> Response {
    let guard = state.last_results.read().await;
    let all_results = guard.clone().unwrap_or_default();
    drop(guard);

    // Filter
    let filtered = filter_results(all_results, &params.lang, &params.topic);

    // Sort
    let sorted = sort_results(filtered, &params.sort, &params.dir);

    // Paginate
    let total_results = sorted.len();
    let page_size = params.page_size.clamp(1, 100);
    let total_pages = if total_results == 0 {
        1
    } else {
        total_results.div_ceil(page_size)
    };
    let page = params.page.max(1).min(total_pages);
    let start = (page - 1) * page_size;
    let end = (start + page_size).min(total_results);
    let page_results: Vec<CrossRefResult> = sorted[start..end].to_vec();

    // Check if HTMX request
    let is_htmx = headers.get("HX-Request").is_some();

    if is_htmx {
        let tmpl = ResultsTableTemplate {
            results: page_results,
            current_sort: params.sort,
            current_dir: params.dir,
            current_lang_filter: params.lang,
            current_page: page,
            total_pages,
        };
        Html(tmpl.render().unwrap_or_else(|e| {
            format!("<tr><td colspan=\"6\">Template error: {e}</td></tr>")
        }))
        .into_response()
    } else {
        Json(ResultsResponse {
            results: page_results,
            page,
            page_size,
            total_pages,
            total_results,
        })
        .into_response()
    }
}

fn filter_results(
    results: Vec<CrossRefResult>,
    lang: &str,
    topic: &str,
) -> Vec<CrossRefResult> {
    results
        .into_iter()
        .filter(|r| {
            if !lang.is_empty() {
                match &r.analysis.candidate.language {
                    Some(l) if l.eq_ignore_ascii_case(lang) => {}
                    _ => return false,
                }
            }
            if !topic.is_empty() {
                let has_topic = r
                    .analysis
                    .candidate
                    .topics
                    .iter()
                    .any(|t| t.eq_ignore_ascii_case(topic));
                if !has_topic {
                    return false;
                }
            }
            true
        })
        .collect()
}

fn sort_results(
    mut results: Vec<CrossRefResult>,
    sort: &str,
    dir: &str,
) -> Vec<CrossRefResult> {
    let ascending = dir == "asc";
    results.sort_by(|a, b| {
        let cmp = match sort {
            "name" => a
                .analysis
                .candidate
                .repo_name
                .to_lowercase()
                .cmp(&b.analysis.candidate.repo_name.to_lowercase()),
            "stars" => a
                .analysis
                .candidate
                .stars
                .cmp(&b.analysis.candidate.stars),
            "relevance" => a
                .overall_relevance
                .partial_cmp(&b.overall_relevance)
                .unwrap_or(std::cmp::Ordering::Equal),
            "language" => {
                let la = a
                    .analysis
                    .candidate
                    .language
                    .as_deref()
                    .unwrap_or("");
                let lb = b
                    .analysis
                    .candidate
                    .language
                    .as_deref()
                    .unwrap_or("");
                la.to_lowercase().cmp(&lb.to_lowercase())
            }
            _ => a
                .analysis
                .candidate
                .stars
                .cmp(&b.analysis.candidate.stars),
        };
        if ascending { cmp } else { cmp.reverse() }
    });
    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::web::router;
    use crate::config::AppConfig;
    use crate::adapters::web::state::ScanStatus;
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

    fn mock_result(name: &str, stars: u64, lang: &str, relevance: f64) -> CrossRefResult {
        CrossRefResult {
            analysis: AnalysisResult {
                candidate: RepoCandidate {
                    entry: FeedEntry {
                        title: name.to_string(),
                        repo_url: Url::parse(&format!("https://github.com/owner/{name}"))
                            .unwrap(),
                        description: Some("A test repo".into()),
                        published: Some(Utc::now()),
                        source_name: "GitHub Trending".into(),
                    },
                    stars,
                    language: Some(lang.to_string()),
                    topics: vec!["cli".into(), "tooling".into()],
                    fork: false,
                    archived: false,
                    owner: "owner".into(),
                    repo_name: name.to_string(),
                    category: Default::default(),
                    semantic_score: 0.0,
                    pushed_at: None,
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

    #[tokio::test]
    async fn api_results_sort_by_stars_desc() {
        let results = vec![
            mock_result("low", 10, "Rust", 0.5),
            mock_result("high", 1000, "Go", 0.9),
            mock_result("mid", 100, "Python", 0.7),
        ];
        let app = router(test_state_with_results(results));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/results?sort=stars&dir=desc")
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
        let resp: ResultsResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(resp.results.len(), 3);
        assert_eq!(resp.results[0].analysis.candidate.stars, 1000);
        assert_eq!(resp.results[1].analysis.candidate.stars, 100);
        assert_eq!(resp.results[2].analysis.candidate.stars, 10);
    }

    #[tokio::test]
    async fn api_results_filter_by_language() {
        let results = vec![
            mock_result("rust-repo", 100, "Rust", 0.8),
            mock_result("go-repo", 200, "Go", 0.6),
            mock_result("rust-repo2", 50, "Rust", 0.9),
        ];
        let app = router(test_state_with_results(results));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/results?lang=Rust")
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
        let resp: ResultsResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(resp.results.len(), 2);
        assert_eq!(resp.total_results, 2);
    }

    #[tokio::test]
    async fn api_results_pagination() {
        let results: Vec<CrossRefResult> = (0..25)
            .map(|i| mock_result(&format!("repo-{i}"), i * 10, "Rust", 0.5))
            .collect();
        let app = router(test_state_with_results(results));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/results?page=2&page_size=10")
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
        let resp: ResultsResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(resp.page, 2);
        assert_eq!(resp.total_pages, 3);
        assert_eq!(resp.results.len(), 10);
    }

    #[tokio::test]
    async fn api_results_json_without_hx_header() {
        let results = vec![mock_result("test", 42, "Rust", 0.8)];
        let app = router(test_state_with_results(results));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/results")
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
        assert!(ct.contains("json"), "should return JSON, got {ct}");
    }

    #[tokio::test]
    async fn api_results_html_with_hx_header() {
        let results = vec![mock_result("test", 42, "Rust", 0.8)];
        let app = router(test_state_with_results(results));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/results")
                    .header("HX-Request", "true")
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
        assert!(ct.contains("html"), "should return HTML, got {ct}");
    }

    #[test]
    fn sort_results_by_name_asc() {
        let results = vec![
            mock_result("zeta", 10, "Rust", 0.5),
            mock_result("alpha", 20, "Go", 0.9),
        ];
        let sorted = sort_results(results, "name", "asc");
        assert_eq!(sorted[0].analysis.candidate.repo_name, "alpha");
        assert_eq!(sorted[1].analysis.candidate.repo_name, "zeta");
    }

    #[test]
    fn filter_results_by_topic() {
        let mut r = mock_result("a", 1, "Rust", 0.5);
        r.analysis.candidate.topics = vec!["web".into()];
        let results = vec![r, mock_result("b", 2, "Go", 0.5)];
        let filtered = filter_results(results, "", "web");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].analysis.candidate.repo_name, "a");
    }

    #[test]
    fn filter_results_empty_params_returns_all() {
        let results = vec![
            mock_result("a", 1, "Rust", 0.5),
            mock_result("b", 2, "Go", 0.5),
        ];
        let filtered = filter_results(results, "", "");
        assert_eq!(filtered.len(), 2);
    }
}
