use askama::Template;
use serde::Serialize;
use std::collections::HashMap;

use crate::domain::model::CrossRefResult;

/// Aggregated statistics for the dashboard view.
#[derive(Debug, Clone, Default, Serialize)]
pub struct DashboardStats {
    pub total_repos: usize,
    pub avg_relevance: f64,
    pub top_languages: Vec<(String, usize)>,
    /// Count of repos in each relevance bucket: [0-20%, 20-40%, 40-60%, 60-80%, 80-100%]
    pub relevance_buckets: [usize; 5],
}

impl DashboardStats {
    /// Compute stats from a slice of cross-ref results.
    pub fn from_results(results: &[CrossRefResult]) -> Self {
        let total_repos = results.len();

        let avg_relevance = if total_repos == 0 {
            0.0
        } else {
            let sum: f64 = results.iter().map(|r| r.overall_relevance).sum();
            sum / total_repos as f64
        };

        let mut lang_counts: HashMap<String, usize> = HashMap::new();
        for result in results {
            if let Some(ref lang) = result.analysis.candidate.language {
                *lang_counts.entry(lang.clone()).or_insert(0) += 1;
            }
        }
        let mut top_languages: Vec<(String, usize)> = lang_counts.into_iter().collect();
        top_languages.sort_by(|a, b| b.1.cmp(&a.1));
        top_languages.truncate(5);

        // Build relevance buckets: [0-20%, 20-40%, 40-60%, 60-80%, 80-100%]
        let mut relevance_buckets = [0usize; 5];
        for result in results {
            let pct = (result.overall_relevance * 100.0).round() as u32;
            let idx = match pct {
                0..=19 => 0,
                20..=39 => 1,
                40..=59 => 2,
                60..=79 => 3,
                _ => 4,
            };
            relevance_buckets[idx] += 1;
        }

        Self {
            total_repos,
            avg_relevance,
            top_languages,
            relevance_buckets,
        }
    }

    /// Return avg_relevance as a percentage string (e.g. "85.0").
    pub fn avg_relevance_pct(&self) -> String {
        format!("{:.1}", self.avg_relevance * 100.0)
    }

    /// Return top languages formatted for display (e.g. "Rust (5), Go (3), Python (2)").
    pub fn top_languages_display(&self) -> String {
        if self.top_languages.is_empty() {
            return "None".to_string();
        }
        self.top_languages
            .iter()
            .map(|(lang, count)| format!("{lang} ({count})"))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// Return relevance as an integer percentage (0-100).
pub fn relevance_pct(relevance: f64) -> u32 {
    (relevance * 100.0).round() as u32
}

/// Return the CSS color class for a relevance percentage.
pub fn relevance_color(relevance: f64) -> &'static str {
    let pct = relevance_pct(relevance);
    if pct >= 80 {
        "text-green-400"
    } else if pct >= 50 {
        "text-yellow-400"
    } else {
        "text-red-400"
    }
}

/// Return the sort direction toggle for a column header.
pub fn toggle_dir(current_sort: &str, current_dir: &str, column: &str, default_dir: &str) -> &'static str {
    if current_sort == column {
        if current_dir == default_dir {
            if default_dir == "asc" { "desc" } else { "asc" }
        } else if default_dir == "asc" {
            "asc"
        } else {
            "desc"
        }
    } else if default_dir == "asc" {
        "asc"
    } else {
        "desc"
    }
}

/// Return the sort indicator arrow for a column.
pub fn sort_indicator(current_sort: &str, current_dir: &str, column: &str) -> &'static str {
    if current_sort == column {
        if current_dir == "asc" { "\u{25B2}" } else { "\u{25BC}" }
    } else {
        ""
    }
}

/// Collect all unique languages from results.
pub fn collect_languages(results: &[CrossRefResult]) -> Vec<String> {
    let mut langs: Vec<String> = results
        .iter()
        .filter_map(|r| r.analysis.candidate.language.clone())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    langs.sort();
    langs
}

/// Main dashboard page template.
#[derive(Template)]
#[template(path = "dashboard.html")]
pub struct DashboardTemplate {
    pub results: Vec<CrossRefResult>,
    pub stats: DashboardStats,
    pub current_sort: String,
    pub current_dir: String,
    pub current_lang_filter: String,
    pub current_page: usize,
    pub total_pages: usize,
    pub all_languages: Vec<String>,
    /// JSON-encoded relevance buckets for Chart.js
    pub chart_relevance_json: String,
    /// JSON-encoded language counts for Chart.js
    pub chart_languages_json: String,
}

impl DashboardTemplate {
    /// Get relevance percentage for a result.
    pub fn rel_pct(&self, relevance: &f64) -> u32 {
        relevance_pct(*relevance)
    }

    /// Get relevance color class for a result.
    pub fn rel_color(&self, relevance: &f64) -> &'static str {
        relevance_color(*relevance)
    }

    /// Get toggle direction for a sort column.
    pub fn toggle(&self, column: &str, default_dir: &str) -> &'static str {
        toggle_dir(&self.current_sort, &self.current_dir, column, default_dir)
    }

    /// Get sort indicator for a column.
    pub fn indicator(&self, column: &str) -> &'static str {
        sort_indicator(&self.current_sort, &self.current_dir, column)
    }

    /// Check if current_lang_filter matches a language.
    pub fn is_lang_selected(&self, lang: &str) -> bool {
        self.current_lang_filter == lang
    }
}

/// Error page template.
#[derive(Template)]
#[template(path = "error.html")]
pub struct ErrorTemplate {
    pub status_code: u16,
    pub title: String,
    pub message: String,
}

/// Partial template for the results table body (HTMX swap target).
#[derive(Template)]
#[template(path = "partials/results_table.html")]
pub struct ResultsTableTemplate {
    pub results: Vec<CrossRefResult>,
    pub current_sort: String,
    pub current_dir: String,
    pub current_lang_filter: String,
    pub current_page: usize,
    pub total_pages: usize,
}

impl ResultsTableTemplate {
    /// Get relevance percentage for a result.
    pub fn rel_pct(&self, relevance: &f64) -> u32 {
        relevance_pct(*relevance)
    }

    /// Get relevance color class for a result.
    pub fn rel_color(&self, relevance: &f64) -> &'static str {
        relevance_color(*relevance)
    }

    /// Get toggle direction for a sort column.
    pub fn toggle(&self, column: &str, default_dir: &str) -> &'static str {
        toggle_dir(&self.current_sort, &self.current_dir, column, default_dir)
    }

    /// Get sort indicator for a column.
    pub fn indicator(&self, column: &str) -> &'static str {
        sort_indicator(&self.current_sort, &self.current_dir, column)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::model::{
        AnalysisResult, CrossRefResult, FeedEntry, RepoCandidate, RepoMatch,
    };
    use askama::Template;
    use chrono::Utc;
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

    #[test]
    fn dashboard_stats_from_empty_results() {
        let stats = DashboardStats::from_results(&[]);
        assert_eq!(stats.total_repos, 0);
        assert!((stats.avg_relevance - 0.0).abs() < f64::EPSILON);
        assert!(stats.top_languages.is_empty());
    }

    #[test]
    fn dashboard_stats_computes_correctly() {
        let results = vec![
            mock_result("alpha", 100, "Rust", 0.8),
            mock_result("beta", 200, "Rust", 0.6),
            mock_result("gamma", 50, "Go", 0.9),
        ];
        let stats = DashboardStats::from_results(&results);
        assert_eq!(stats.total_repos, 3);
        let expected_avg = (0.8 + 0.6 + 0.9) / 3.0;
        assert!((stats.avg_relevance - expected_avg).abs() < 1e-10);
        assert_eq!(stats.top_languages[0], ("Rust".to_string(), 2));
        assert_eq!(stats.top_languages[1], ("Go".to_string(), 1));
    }

    #[test]
    fn dashboard_stats_avg_relevance_pct() {
        let stats = DashboardStats {
            avg_relevance: 0.85,
            ..Default::default()
        };
        assert_eq!(stats.avg_relevance_pct(), "85.0");
    }

    #[test]
    fn dashboard_stats_top_languages_display() {
        let stats = DashboardStats {
            top_languages: vec![("Rust".into(), 5), ("Go".into(), 3)],
            ..Default::default()
        };
        assert_eq!(stats.top_languages_display(), "Rust (5), Go (3)");
    }

    #[test]
    fn dashboard_stats_top_languages_display_empty() {
        let stats = DashboardStats::default();
        assert_eq!(stats.top_languages_display(), "None");
    }

    #[test]
    fn collect_languages_returns_sorted_unique() {
        let results = vec![
            mock_result("a", 1, "Rust", 0.5),
            mock_result("b", 2, "Go", 0.5),
            mock_result("c", 3, "Rust", 0.5),
        ];
        let langs = collect_languages(&results);
        assert_eq!(langs, vec!["Go", "Rust"]);
    }

    #[test]
    fn dashboard_template_renders_valid_html() {
        let results = vec![mock_result("test-repo", 42, "Rust", 0.85)];
        let stats = DashboardStats::from_results(&results);
        let langs = collect_languages(&results);
        let chart_relevance_json = serde_json::to_string(&stats.relevance_buckets).unwrap();
        let chart_languages_json = serde_json::to_string(&stats.top_languages).unwrap();
        let tmpl = DashboardTemplate {
            results,
            stats,
            current_sort: "stars".into(),
            current_dir: "desc".into(),
            current_lang_filter: String::new(),
            current_page: 1,
            total_pages: 1,
            all_languages: langs,
            chart_relevance_json,
            chart_languages_json,
        };
        let rendered = tmpl.render().unwrap();
        assert!(rendered.contains("<!DOCTYPE html>"));
        assert!(rendered.contains("test-repo"));
        assert!(rendered.contains("42"));
    }

    #[test]
    fn results_table_template_renders_rows() {
        let results = vec![mock_result("table-repo", 99, "Python", 0.72)];
        let tmpl = ResultsTableTemplate {
            results,
            current_sort: "stars".into(),
            current_dir: "desc".into(),
            current_lang_filter: String::new(),
            current_page: 1,
            total_pages: 1,
        };
        let rendered = tmpl.render().unwrap();
        assert!(rendered.contains("table-repo"));
        assert!(rendered.contains("99"));
        assert!(rendered.contains("Python"));
    }
}
