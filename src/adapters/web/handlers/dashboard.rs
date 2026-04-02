use askama::Template;
use axum::extract::State;
use axum::response::Html;

use crate::adapters::web::AppState;
use crate::adapters::web::templates::{
    DashboardStats, DashboardTemplate, collect_languages,
};

/// GET / — render the main dashboard page.
pub async fn index(State(state): State<AppState>) -> Html<String> {
    let guard = state.last_results.read().await;
    let results = guard.clone().unwrap_or_default();
    drop(guard);

    let stats = DashboardStats::from_results(&results);
    let all_languages = collect_languages(&results);
    let total_pages = 1; // full render shows all results on one page

    let chart_relevance_json = serde_json::to_string(&stats.relevance_buckets)
        .unwrap_or_else(|_| "[0,0,0,0,0]".to_string());
    let chart_languages_json = serde_json::to_string(&stats.top_languages)
        .unwrap_or_else(|_| "[]".to_string());

    let tmpl = DashboardTemplate {
        results,
        stats,
        current_sort: "stars".into(),
        current_dir: "desc".into(),
        current_lang_filter: String::new(),
        current_page: 1,
        total_pages,
        all_languages,
        chart_relevance_json,
        chart_languages_json,
    };

    Html(tmpl.render().unwrap_or_else(|e| {
        format!("<h1>Template error</h1><p>{e}</p>")
    }))
}
