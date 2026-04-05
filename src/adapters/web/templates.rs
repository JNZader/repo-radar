use askama::Template;
use serde::Serialize;
use std::collections::HashMap;

use crate::domain::diff::ScanDiff;
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

// ── Comparison view ─────────────────────────────────────────────────

/// Discovered repo details for the comparison view.
#[derive(Debug, Clone, Serialize)]
pub struct DiscoveredRepo {
    pub owner: String,
    pub repo_name: String,
    pub repo_url: String,
    pub stars: u64,
    pub language: Option<String>,
    pub topics: Vec<String>,
    pub description: Option<String>,
    pub summary: String,
    pub key_features: Vec<String>,
    pub tech_stack: Vec<String>,
    pub relevance_score: f64,
    pub overall_relevance: f64,
}

/// A single match detail for the comparison view.
#[derive(Debug, Clone, Serialize)]
pub struct MatchDetail {
    pub own_repo: String,
    pub relevance: f64,
    pub reason: String,
}

impl MatchDetail {
    /// Return relevance as an integer percentage.
    pub fn rel_pct(&self) -> u32 {
        relevance_pct(self.relevance)
    }

    /// Return the CSS color class for relevance.
    pub fn rel_color(&self) -> &'static str {
        relevance_color(self.relevance)
    }
}

/// Build comparison data from a CrossRefResult.
pub fn build_compare_data(result: &CrossRefResult) -> (DiscoveredRepo, Vec<MatchDetail>, Vec<String>) {
    let candidate = &result.analysis.candidate;

    let discovered = DiscoveredRepo {
        owner: candidate.owner.clone(),
        repo_name: candidate.repo_name.clone(),
        repo_url: candidate.entry.repo_url.to_string(),
        stars: candidate.stars,
        language: candidate.language.clone(),
        topics: candidate.topics.clone(),
        description: candidate.entry.description.clone(),
        summary: result.analysis.summary.clone(),
        key_features: result.analysis.key_features.clone(),
        tech_stack: result.analysis.tech_stack.clone(),
        relevance_score: result.analysis.relevance_score,
        overall_relevance: result.overall_relevance,
    };

    let matches: Vec<MatchDetail> = result
        .matched_repos
        .iter()
        .map(|m| MatchDetail {
            own_repo: m.own_repo.clone(),
            relevance: m.relevance,
            reason: m.reason.clone(),
        })
        .collect();

    // Extract shared topics from match reasons to find unique ones
    let mut shared_topics: std::collections::HashSet<String> = std::collections::HashSet::new();
    for m in &result.matched_repos {
        // Parse "shared topics: x, y" from reason string
        if let Some(topics_part) = m.reason.split("shared topics: ").nth(1) {
            for topic in topics_part.split(", ") {
                let topic = topic.split(';').next().unwrap_or(topic).trim();
                if !topic.is_empty() {
                    shared_topics.insert(topic.to_string());
                }
            }
        }
    }

    let unique_topics: Vec<String> = candidate
        .topics
        .iter()
        .filter(|t| !shared_topics.contains(&t.to_ascii_lowercase()))
        .cloned()
        .collect();

    (discovered, matches, unique_topics)
}

/// Comparison page template.
#[derive(Template)]
#[template(path = "compare.html")]
pub struct CompareTemplate {
    pub discovered: DiscoveredRepo,
    pub matches: Vec<MatchDetail>,
    pub unique_topics: Vec<String>,
}

impl CompareTemplate {
    /// Get relevance percentage for the discovered repo.
    pub fn rel_pct(&self, relevance: &f64) -> u32 {
        relevance_pct(*relevance)
    }

    /// Get relevance color class.
    pub fn rel_color(&self, relevance: &f64) -> &'static str {
        relevance_color(*relevance)
    }
}

// ── Diff view ────────────────────────────────────────────────────────────────

/// Full-page diff view template.
#[derive(Template)]
#[template(path = "diff.html")]
pub struct DiffTemplate {
    pub diff: ScanDiff,
}

impl DiffTemplate {
    /// Format a score delta as a signed percentage string.
    pub fn fmt_delta(&self, delta: &f64) -> String {
        fmt_score_delta(*delta)
    }

    /// CSS color class for a score delta.
    pub fn delta_color(&self, delta: &f64) -> &'static str {
        score_delta_color(*delta)
    }

    pub fn rel_pct(&self, relevance: &f64) -> u32 {
        relevance_pct(*relevance)
    }
}

/// HTMX-swappable diff table partial template.
#[derive(Template)]
#[template(path = "partials/diff_table.html")]
pub struct DiffTableTemplate {
    pub diff: ScanDiff,
}

impl DiffTableTemplate {
    pub fn fmt_delta(&self, delta: &f64) -> String {
        fmt_score_delta(*delta)
    }

    pub fn delta_color(&self, delta: &f64) -> &'static str {
        score_delta_color(*delta)
    }

    pub fn rel_pct(&self, relevance: &f64) -> u32 {
        relevance_pct(*relevance)
    }
}

/// Format a score delta as a signed percentage string (e.g. "+12.3%" or "-5.0%").
pub fn fmt_score_delta(delta: f64) -> String {
    if delta >= 0.0 {
        format!("+{:.1}%", delta * 100.0)
    } else {
        format!("{:.1}%", delta * 100.0)
    }
}

/// CSS color class for a score delta value.
pub fn score_delta_color(delta: f64) -> &'static str {
    if delta >= 0.05 {
        "text-green-400"
    } else if delta <= -0.05 {
        "text-red-400"
    } else {
        "text-gray-400"
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

    // ── relevance_pct ─────────────────────────────────────────────────────────

    #[test]
    fn relevance_pct_boundary_values() {
        assert_eq!(relevance_pct(0.0), 0);
        assert_eq!(relevance_pct(0.5), 50);
        assert_eq!(relevance_pct(0.8), 80);
        assert_eq!(relevance_pct(1.0), 100);
    }

    #[test]
    fn relevance_pct_rounding() {
        // 0.555 * 100 = 55.5 → rounds to 56
        assert_eq!(relevance_pct(0.555), 56);
        // 0.554 * 100 = 55.4 → rounds to 55
        assert_eq!(relevance_pct(0.554), 55);
    }

    #[test]
    fn relevance_pct_out_of_range_values() {
        // Negative input: wraps via saturating cast — f64 negative casts to 0 in Rust
        // Actually (−0.5 * 100.0).round() as u32 wraps; we just verify it doesn't panic.
        let _ = relevance_pct(-0.5);
        // Values > 1.0 produce > 100 — no panic expected
        assert!(relevance_pct(1.5) > 100);
    }

    // ── relevance_color ───────────────────────────────────────────────────────

    #[test]
    fn relevance_color_thresholds() {
        // pct >= 80 → green
        assert_eq!(relevance_color(0.80), "text-green-400");
        assert_eq!(relevance_color(1.0), "text-green-400");
        assert_eq!(relevance_color(0.81), "text-green-400");

        // pct in [50, 79] → yellow
        assert_eq!(relevance_color(0.50), "text-yellow-400");
        assert_eq!(relevance_color(0.79), "text-yellow-400");
        assert_eq!(relevance_color(0.65), "text-yellow-400");

        // pct < 50 → red
        assert_eq!(relevance_color(0.49), "text-red-400");
        assert_eq!(relevance_color(0.0), "text-red-400");
        assert_eq!(relevance_color(0.20), "text-red-400");
    }

    #[test]
    fn relevance_color_exact_boundary_between_green_and_yellow() {
        // 0.799... rounds to 80 → green
        assert_eq!(relevance_color(0.7999), "text-green-400");
        // 0.795 rounds to 80 → green
        assert_eq!(relevance_color(0.795), "text-green-400");
        // 0.794 rounds to 79 → yellow
        assert_eq!(relevance_color(0.794), "text-yellow-400");
    }

    #[test]
    fn relevance_color_exact_boundary_between_yellow_and_red() {
        // 0.495 rounds to 50 → yellow
        assert_eq!(relevance_color(0.495), "text-yellow-400");
        // 0.494 rounds to 49 → red
        assert_eq!(relevance_color(0.494), "text-red-400");
    }

    // ── toggle_dir ────────────────────────────────────────────────────────────

    #[test]
    fn toggle_dir_different_column_returns_default() {
        // Not the active column → return default_dir regardless
        assert_eq!(toggle_dir("stars", "desc", "relevance", "desc"), "desc");
        assert_eq!(toggle_dir("stars", "asc", "relevance", "asc"), "asc");
        assert_eq!(toggle_dir("stars", "desc", "relevance", "asc"), "asc");
    }

    #[test]
    fn toggle_dir_same_column_flips_from_default() {
        // current is default_dir=desc, same column → flip to asc
        assert_eq!(toggle_dir("stars", "desc", "stars", "desc"), "asc");
        // current is default_dir=asc, same column → flip to desc
        assert_eq!(toggle_dir("stars", "asc", "stars", "asc"), "desc");
    }

    #[test]
    fn toggle_dir_same_column_already_flipped_returns_default() {
        // default=desc, current=asc (already flipped) → return desc (default)
        assert_eq!(toggle_dir("stars", "asc", "stars", "desc"), "desc");
        // default=asc, current=desc (already flipped) → return asc (default)
        assert_eq!(toggle_dir("stars", "desc", "stars", "asc"), "asc");
    }

    // ── sort_indicator ────────────────────────────────────────────────────────

    #[test]
    fn sort_indicator_active_column_asc() {
        assert_eq!(sort_indicator("stars", "asc", "stars"), "\u{25B2}");
    }

    #[test]
    fn sort_indicator_active_column_desc() {
        assert_eq!(sort_indicator("stars", "desc", "stars"), "\u{25BC}");
    }

    #[test]
    fn sort_indicator_inactive_column_is_empty() {
        assert_eq!(sort_indicator("stars", "asc", "relevance"), "");
        assert_eq!(sort_indicator("stars", "desc", "name"), "");
    }

    // ── fmt_score_delta ───────────────────────────────────────────────────────

    #[test]
    fn fmt_score_delta_positive() {
        assert_eq!(fmt_score_delta(0.123), "+12.3%");
        assert_eq!(fmt_score_delta(1.0), "+100.0%");
    }

    #[test]
    fn fmt_score_delta_negative() {
        assert_eq!(fmt_score_delta(-0.05), "-5.0%");
        assert_eq!(fmt_score_delta(-0.123), "-12.3%");
    }

    #[test]
    fn fmt_score_delta_zero() {
        assert_eq!(fmt_score_delta(0.0), "+0.0%");
    }

    #[test]
    fn fmt_score_delta_very_small_positive() {
        // 0.001 * 100 = 0.1 → "+0.1%"
        assert_eq!(fmt_score_delta(0.001), "+0.1%");
    }

    #[test]
    fn fmt_score_delta_very_small_negative() {
        assert_eq!(fmt_score_delta(-0.001), "-0.1%");
    }

    // ── score_delta_color ─────────────────────────────────────────────────────

    #[test]
    fn score_delta_color_positive_above_threshold() {
        assert_eq!(score_delta_color(0.05), "text-green-400");
        assert_eq!(score_delta_color(0.10), "text-green-400");
        assert_eq!(score_delta_color(1.0), "text-green-400");
    }

    #[test]
    fn score_delta_color_negative_above_threshold() {
        assert_eq!(score_delta_color(-0.05), "text-red-400");
        assert_eq!(score_delta_color(-0.10), "text-red-400");
        assert_eq!(score_delta_color(-1.0), "text-red-400");
    }

    #[test]
    fn score_delta_color_within_neutral_band() {
        assert_eq!(score_delta_color(0.0), "text-gray-400");
        assert_eq!(score_delta_color(0.04), "text-gray-400");
        assert_eq!(score_delta_color(-0.04), "text-gray-400");
    }

    #[test]
    fn score_delta_color_exact_boundary_positive() {
        // exactly 0.05 → green (>= 0.05)
        assert_eq!(score_delta_color(0.05), "text-green-400");
        // just below → gray
        assert_eq!(score_delta_color(0.049), "text-gray-400");
    }

    #[test]
    fn score_delta_color_exact_boundary_negative() {
        // exactly -0.05 → red (<= -0.05)
        assert_eq!(score_delta_color(-0.05), "text-red-400");
        // just above threshold → gray
        assert_eq!(score_delta_color(-0.049), "text-gray-400");
    }

    // ── build_compare_data ────────────────────────────────────────────────────

    fn make_crossref_result(
        owner: &str,
        repo_name: &str,
        relevance: f64,
        topics: Vec<String>,
        matches: Vec<RepoMatch>,
    ) -> CrossRefResult {
        CrossRefResult {
            analysis: AnalysisResult {
                candidate: RepoCandidate {
                    entry: FeedEntry {
                        title: repo_name.to_string(),
                        repo_url: Url::parse(&format!("https://github.com/{owner}/{repo_name}"))
                            .unwrap(),
                        description: Some("Test repo".into()),
                        published: Some(Utc::now()),
                        source_name: "test".into(),
                    },
                    stars: 42,
                    language: Some("Rust".into()),
                    topics,
                    fork: false,
                    archived: false,
                    owner: owner.to_string(),
                    repo_name: repo_name.to_string(),
                    category: Default::default(),
                },
                summary: "A test summary".into(),
                key_features: vec!["feature-a".into()],
                tech_stack: vec!["Rust".into()],
                relevance_score: relevance,
            },
            matched_repos: matches,
            ideas: vec!["idea-1".into()],
            overall_relevance: relevance,
        }
    }

    #[test]
    fn build_compare_data_basic_fields() {
        let result = make_crossref_result(
            "acme",
            "my-tool",
            0.75,
            vec!["cli".into(), "automation".into()],
            vec![RepoMatch {
                own_repo: "user/project".into(),
                relevance: 0.8,
                reason: "Similar tech".into(),
            }],
        );

        let (discovered, matches, _unique_topics) = build_compare_data(&result);

        assert_eq!(discovered.owner, "acme");
        assert_eq!(discovered.repo_name, "my-tool");
        assert_eq!(discovered.repo_url, "https://github.com/acme/my-tool");
        assert_eq!(discovered.stars, 42);
        assert_eq!(discovered.language, Some("Rust".into()));
        assert!((discovered.overall_relevance - 0.75).abs() < f64::EPSILON);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].own_repo, "user/project");
        assert!((matches[0].relevance - 0.8).abs() < f64::EPSILON);
    }

    #[test]
    fn build_compare_data_no_matches_returns_empty_vec() {
        let result = make_crossref_result("owner", "repo", 0.5, vec![], vec![]);
        let (_discovered, matches, unique_topics) = build_compare_data(&result);
        assert!(matches.is_empty());
        assert!(unique_topics.is_empty());
    }

    #[test]
    fn build_compare_data_unique_topics_excludes_shared() {
        // Topics on the repo
        let topics = vec!["rust".into(), "cli".into(), "exclusive-topic".into()];
        // Match reason mentions "shared topics: rust, cli"
        let matches = vec![RepoMatch {
            own_repo: "user/my-repo".into(),
            relevance: 0.9,
            reason: "shared topics: rust, cli; also very fast".into(),
        }];
        let result = make_crossref_result("owner", "repo", 0.8, topics, matches);
        let (_discovered, _matches, unique_topics) = build_compare_data(&result);

        // "exclusive-topic" is not shared so it should appear
        assert!(
            unique_topics.contains(&"exclusive-topic".to_string()),
            "unique_topics should contain 'exclusive-topic', got: {unique_topics:?}"
        );
        // "rust" and "cli" are shared and should NOT appear
        assert!(
            !unique_topics.contains(&"rust".to_string()),
            "shared topic 'rust' should not be in unique_topics"
        );
        assert!(
            !unique_topics.contains(&"cli".to_string()),
            "shared topic 'cli' should not be in unique_topics"
        );
    }

    #[test]
    fn build_compare_data_multiple_matches_all_preserved() {
        let matches = vec![
            RepoMatch { own_repo: "user/a".into(), relevance: 0.9, reason: "reason-a".into() },
            RepoMatch { own_repo: "user/b".into(), relevance: 0.7, reason: "reason-b".into() },
            RepoMatch { own_repo: "user/c".into(), relevance: 0.5, reason: "reason-c".into() },
        ];
        let result = make_crossref_result("owner", "repo", 0.8, vec![], matches);
        let (_discovered, match_details, _unique_topics) = build_compare_data(&result);
        assert_eq!(match_details.len(), 3);
    }

    #[test]
    fn match_detail_rel_pct_and_color() {
        let md = MatchDetail {
            own_repo: "test".into(),
            relevance: 0.85,
            reason: "test".into(),
        };
        assert_eq!(md.rel_pct(), 85);
        assert_eq!(md.rel_color(), "text-green-400");

        let md_low = MatchDetail {
            own_repo: "test".into(),
            relevance: 0.3,
            reason: "test".into(),
        };
        assert_eq!(md_low.rel_pct(), 30);
        assert_eq!(md_low.rel_color(), "text-red-400");
    }

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
