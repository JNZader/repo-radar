use askama::Template;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Json, Response};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::adapters::web::AppState;
use crate::config::AppConfig;

// ── Config page ─────────────────────────────────────────────────────

/// A single config entry for display.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigEntry {
    pub key: String,
    pub value: String,
}

/// A group of related config entries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigSection {
    pub title: String,
    pub entries: Vec<ConfigEntry>,
}

/// Mask sensitive values so they never leak to the UI.
fn mask_secret(value: &Option<String>) -> String {
    match value {
        Some(_) => "[set via env]".to_string(),
        None => "not set".to_string(),
    }
}

/// Build displayable config sections from `AppConfig`.
fn build_config_sections(config: &AppConfig) -> Vec<ConfigSection> {
    let mut sections = Vec::new();

    // Feeds
    let feed_entries: Vec<ConfigEntry> = if config.feeds.is_empty() {
        vec![ConfigEntry {
            key: "(none configured)".into(),
            value: String::new(),
        }]
    } else {
        config
            .feeds
            .iter()
            .enumerate()
            .map(|(i, f)| ConfigEntry {
                key: f.name.clone().unwrap_or_else(|| format!("Feed {}", i + 1)),
                value: f.url.clone(),
            })
            .collect()
    };
    sections.push(ConfigSection {
        title: "Feeds".into(),
        entries: feed_entries,
    });

    // Filter
    sections.push(ConfigSection {
        title: "Filter".into(),
        entries: vec![
            ConfigEntry {
                key: "min_stars".into(),
                value: config.filter.min_stars.to_string(),
            },
            ConfigEntry {
                key: "languages".into(),
                value: if config.filter.languages.is_empty() {
                    "any".into()
                } else {
                    config.filter.languages.join(", ")
                },
            },
            ConfigEntry {
                key: "topics".into(),
                value: if config.filter.topics.is_empty() {
                    "any".into()
                } else {
                    config.filter.topics.join(", ")
                },
            },
            ConfigEntry {
                key: "exclude_forks".into(),
                value: config.filter.exclude_forks.to_string(),
            },
            ConfigEntry {
                key: "exclude_archived".into(),
                value: config.filter.exclude_archived.to_string(),
            },
        ],
    });

    // Analyzer
    sections.push(ConfigSection {
        title: "Analyzer".into(),
        entries: vec![
            ConfigEntry {
                key: "repoforge_path".into(),
                value: config
                    .analyzer
                    .repoforge_path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "not set".into()),
            },
            ConfigEntry {
                key: "timeout_secs".into(),
                value: config.analyzer.timeout_secs.to_string(),
            },
            ConfigEntry {
                key: "llm_model".into(),
                value: config
                    .analyzer
                    .llm_model
                    .clone()
                    .unwrap_or_else(|| "not set".into()),
            },
            ConfigEntry {
                key: "llm_api_key".into(),
                value: mask_secret(&config.analyzer.llm_api_key),
            },
        ],
    });

    // Cross-reference
    sections.push(ConfigSection {
        title: "Cross-Reference".into(),
        entries: vec![
            ConfigEntry {
                key: "own_repos".into(),
                value: if config.crossref.own_repos.is_empty() {
                    "none".into()
                } else {
                    config
                        .crossref
                        .own_repos
                        .iter()
                        .map(|p| p.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                },
            },
            ConfigEntry {
                key: "github_username".into(),
                value: mask_secret(&config.crossref.github_username),
            },
        ],
    });

    // Reporter
    sections.push(ConfigSection {
        title: "Reporter".into(),
        entries: vec![
            ConfigEntry {
                key: "output_dir".into(),
                value: config.reporter.output_dir.display().to_string(),
            },
            ConfigEntry {
                key: "format".into(),
                value: config.reporter.format.clone(),
            },
        ],
    });

    // General
    sections.push(ConfigSection {
        title: "General".into(),
        entries: vec![
            ConfigEntry {
                key: "data_dir".into(),
                value: config.general.data_dir.display().to_string(),
            },
            ConfigEntry {
                key: "log_level".into(),
                value: config.general.log_level.clone(),
            },
            ConfigEntry {
                key: "backfill_batch_size".into(),
                value: config.general.backfill_batch_size.to_string(),
            },
            ConfigEntry {
                key: "github_token".into(),
                value: mask_secret(&config.general.github_token),
            },
        ],
    });

    sections
}

#[derive(Template)]
#[template(path = "config.html")]
pub struct ConfigTemplate {
    pub sections: Vec<ConfigSection>,
}

/// GET /config — render the config viewer page.
pub async fn config_page(State(state): State<AppState>) -> Html<String> {
    let sections = build_config_sections(&state.config);
    let tmpl = ConfigTemplate { sections };
    Html(tmpl.render().unwrap_or_else(|e| {
        format!("<h1>Template error</h1><p>{e}</p>")
    }))
}

/// GET /api/config — return config as JSON (secrets masked).
pub async fn config_json(State(state): State<AppState>) -> Json<Vec<ConfigSection>> {
    let sections = build_config_sections(&state.config);
    Json(sections)
}

// ── Reports list page ───────────────────────────────────────────────

/// A single report entry for the list view.
#[derive(Debug, Clone, Serialize)]
pub struct ReportEntry {
    pub filename: String,
    pub date: String,
    pub size: String,
    pub format: String,
}

#[derive(Template)]
#[template(path = "reports.html")]
pub struct ReportsTemplate {
    pub reports: Vec<ReportEntry>,
    pub has_reports: bool,
}

/// Format a file size in bytes into a human-readable string.
fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

/// List report files from the configured output directory.
fn list_reports(output_dir: &PathBuf) -> Vec<ReportEntry> {
    let Ok(entries) = std::fs::read_dir(output_dir) else {
        return Vec::new();
    };

    let mut reports: Vec<(std::time::SystemTime, ReportEntry)> = entries
        .filter_map(|e| e.ok())
        .filter_map(|entry| {
            let path = entry.path();
            if !path.is_file() {
                return None;
            }
            let ext = path.extension()?.to_str()?;
            if !matches!(ext, "json" | "md" | "markdown") {
                return None;
            }
            let meta = entry.metadata().ok()?;
            let modified = meta.modified().ok()?;
            let date = modified
                .duration_since(std::time::UNIX_EPOCH)
                .ok()
                .map(|d| {
                    chrono::DateTime::from_timestamp(d.as_secs() as i64, 0)
                        .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                        .unwrap_or_else(|| "unknown".into())
                })
                .unwrap_or_else(|| "unknown".into());
            let filename = path.file_name()?.to_str()?.to_string();
            let format = ext.to_string();
            let size = format_size(meta.len());
            Some((
                modified,
                ReportEntry {
                    filename,
                    date,
                    size,
                    format,
                },
            ))
        })
        .collect();

    // Sort by modification date descending (newest first)
    reports.sort_by(|a, b| b.0.cmp(&a.0));
    reports.into_iter().map(|(_, entry)| entry).collect()
}

/// GET /reports — render the reports list page.
pub async fn reports_page(State(state): State<AppState>) -> Html<String> {
    let reports = list_reports(&state.config.reporter.output_dir);
    let has_reports = !reports.is_empty();
    let tmpl = ReportsTemplate {
        reports,
        has_reports,
    };
    Html(tmpl.render().unwrap_or_else(|e| {
        format!("<h1>Template error</h1><p>{e}</p>")
    }))
}

// ── Report detail page ──────────────────────────────────────────────

#[derive(Template)]
#[template(path = "report_detail.html")]
pub struct ReportDetailTemplate {
    pub filename: String,
    pub content: String,
    pub format: String,
    pub is_json: bool,
}

/// GET /reports/:id — render a single report.
pub async fn report_detail(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let output_dir = &state.config.reporter.output_dir;

    // Sanitize: prevent path traversal
    if id.contains("..") || id.contains('/') || id.contains('\\') {
        return (StatusCode::BAD_REQUEST, Html("Invalid report ID".to_string())).into_response();
    }

    // Try known extensions
    let extensions = ["json", "md", "markdown"];
    let mut found_path = None;

    for ext in &extensions {
        let candidate = output_dir.join(format!("{id}.{ext}"));
        if candidate.is_file() {
            found_path = Some(candidate);
            break;
        }
    }

    // Also try the id as-is (if it already has an extension)
    if found_path.is_none() {
        let direct = output_dir.join(&id);
        if direct.is_file() {
            found_path = Some(direct);
        }
    }

    let Some(path) = found_path else {
        return (
            StatusCode::NOT_FOUND,
            Html("<h1>Report not found</h1>".to_string()),
        )
            .into_response();
    };

    let Ok(raw_content) = std::fs::read_to_string(&path) else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Html("<h1>Could not read report</h1>".to_string()),
        )
            .into_response();
    };

    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("txt");
    let format = ext.to_string();
    let is_json = ext == "json";

    let content = if is_json {
        // Pretty-print JSON
        serde_json::from_str::<serde_json::Value>(&raw_content)
            .map(|v| serde_json::to_string_pretty(&v).unwrap_or_else(|_| raw_content.clone()))
            .unwrap_or(raw_content)
    } else {
        raw_content
    };

    let filename = path
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or(&id)
        .to_string();

    let tmpl = ReportDetailTemplate {
        filename,
        content,
        format,
        is_json,
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

    fn test_state_with_output_dir(dir: PathBuf) -> AppState {
        let (progress_tx, _) = broadcast::channel(16);
        let mut config = AppConfig::default();
        config.reporter.output_dir = dir.clone();
        AppState {
            config,
            scan_status: Arc::new(Mutex::new(ScanStatus::default())),
            last_results: Arc::new(RwLock::new(None)),
            progress_tx,
            scan_store: Arc::new(crate::infra::scan_store::ScanResultStore::new(
                dir.join("results"),
            )),
        }
    }

    #[tokio::test]
    async fn config_page_returns_html() {
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

        assert_eq!(response.status(), 200);
        let body = BodyExt::collect(response.into_body())
            .await
            .unwrap()
            .to_bytes();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(text.contains("<!DOCTYPE html>"));
        assert!(text.contains("Configuration"));
        assert!(text.contains("Filter"));
        assert!(text.contains("Reporter"));
    }

    #[tokio::test]
    async fn config_json_returns_sections() {
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

        assert_eq!(response.status(), 200);
        let ct = response
            .headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        assert!(ct.contains("json"), "should return JSON, got {ct}");

        let body = BodyExt::collect(response.into_body())
            .await
            .unwrap()
            .to_bytes();
        let sections: Vec<ConfigSection> = serde_json::from_slice(&body).unwrap();
        assert!(!sections.is_empty());
        assert_eq!(sections[0].title, "Feeds");
    }

    #[tokio::test]
    async fn config_masks_secrets() {
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

        let body = BodyExt::collect(response.into_body())
            .await
            .unwrap()
            .to_bytes();
        let sections: Vec<ConfigSection> = serde_json::from_slice(&body).unwrap();

        // Check github_token is masked
        let general = sections.iter().find(|s| s.title == "General").unwrap();
        let token_entry = general
            .entries
            .iter()
            .find(|e| e.key == "github_token")
            .unwrap();
        assert_eq!(token_entry.value, "not set");
        assert!(!token_entry.value.contains("secret"));
    }

    #[tokio::test]
    async fn reports_page_empty_output_dir() {
        let dir = tempfile::tempdir().unwrap();
        let app = router(test_state_with_output_dir(dir.path().to_path_buf()));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/reports")
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
        assert!(text.contains("No reports yet"));
    }

    #[tokio::test]
    async fn reports_page_nonexistent_dir() {
        let state = test_state_with_output_dir(PathBuf::from("/tmp/nonexistent_repo_radar_test"));
        let app = router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/reports")
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
        assert!(text.contains("No reports yet"));
    }

    #[tokio::test]
    async fn reports_page_lists_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("report-2024.json"), r#"{"test": true}"#).unwrap();
        std::fs::write(dir.path().join("report-2024.md"), "# Report").unwrap();

        let app = router(test_state_with_output_dir(dir.path().to_path_buf()));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/reports")
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
        assert!(text.contains("report-2024.json"));
        assert!(text.contains("report-2024.md"));
    }

    #[tokio::test]
    async fn report_detail_json() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("my-report.json"),
            r#"{"repos": [{"name": "test"}]}"#,
        )
        .unwrap();

        let app = router(test_state_with_output_dir(dir.path().to_path_buf()));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/reports/my-report")
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
        assert!(text.contains("my-report.json"));
        assert!(text.contains("test"));
    }

    #[tokio::test]
    async fn report_detail_markdown() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("weekly.md"), "# Weekly Report\n\nSome content").unwrap();

        let app = router(test_state_with_output_dir(dir.path().to_path_buf()));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/reports/weekly")
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
        assert!(text.contains("Weekly Report"));
    }

    #[tokio::test]
    async fn report_detail_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let app = router(test_state_with_output_dir(dir.path().to_path_buf()));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/reports/nonexistent")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), 404);
    }

    #[tokio::test]
    async fn report_detail_rejects_path_traversal() {
        let dir = tempfile::tempdir().unwrap();
        let app = router(test_state_with_output_dir(dir.path().to_path_buf()));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/reports/..%2F..%2Fetc%2Fpasswd")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Should be 400 or 404, not 200
        assert_ne!(response.status(), 200);
    }

    #[test]
    fn mask_secret_hides_set_value() {
        assert_eq!(mask_secret(&Some("my-token".into())), "[set via env]");
    }

    #[test]
    fn mask_secret_shows_not_set() {
        assert_eq!(mask_secret(&None), "not set");
    }

    #[test]
    fn format_size_bytes() {
        assert_eq!(format_size(500), "500 B");
    }

    #[test]
    fn format_size_kilobytes() {
        assert_eq!(format_size(2048), "2.0 KB");
    }

    #[test]
    fn format_size_megabytes() {
        assert_eq!(format_size(2 * 1024 * 1024), "2.0 MB");
    }

    #[test]
    fn build_config_sections_covers_all_groups() {
        let config = AppConfig::default();
        let sections = build_config_sections(&config);
        let titles: Vec<&str> = sections.iter().map(|s| s.title.as_str()).collect();
        assert!(titles.contains(&"Feeds"));
        assert!(titles.contains(&"Filter"));
        assert!(titles.contains(&"Analyzer"));
        assert!(titles.contains(&"Cross-Reference"));
        assert!(titles.contains(&"Reporter"));
        assert!(titles.contains(&"General"));
    }

    #[test]
    fn list_reports_ignores_non_report_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("report.json"), "{}").unwrap();
        std::fs::write(dir.path().join("notes.txt"), "hello").unwrap();
        std::fs::write(dir.path().join("image.png"), [0u8; 10]).unwrap();

        let reports = list_reports(&dir.path().to_path_buf());
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].filename, "report.json");
    }
}
