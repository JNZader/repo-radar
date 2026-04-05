use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use url::Url;

use crate::infra::error::PipelineError;

/// Top-level application configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub feeds: Vec<FeedConfig>,
    #[serde(default)]
    pub sources: Vec<SourceConfig>,
    #[serde(default)]
    pub filter: FilterConfig,
    #[serde(default)]
    pub analyzer: AnalyzerConfig,
    #[serde(default)]
    pub crossref: CrossRefConfig,
    #[serde(default)]
    pub reporter: ReporterConfig,
    #[serde(default)]
    pub general: GeneralConfig,
    #[serde(default)]
    pub cache: CacheConfig,
}

const VALID_FORMATS: &[&str] = &["markdown", "json", "console"];
const VALID_LOG_LEVELS: &[&str] = &["trace", "debug", "info", "warn", "error"];
const MAX_TIMEOUT_SECS: u64 = 600;
const MAX_BATCH_SIZE: usize = 1000;

impl AppConfig {
    /// Validate all config values after parsing.
    ///
    /// Collects every problem into a single error so the user can fix them all
    /// at once instead of playing whack-a-mole one at a time.
    pub fn validate(&self) -> Result<(), PipelineError> {
        let mut errors: Vec<String> = Vec::new();

        // 1. Feed URLs
        for (i, feed) in self.feeds.iter().enumerate() {
            if Url::parse(&feed.url).is_err() {
                let fallback = format!("feeds[{i}]");
                let label = feed.name.as_deref().unwrap_or(&fallback);
                errors.push(format!("feed '{label}' has invalid URL: {}", feed.url));
            }
        }

        // 1b. Source configs
        for (i, source) in self.sources.iter().enumerate() {
            match source {
                SourceConfig::Rss { url, name, .. } => {
                    if Url::parse(url).is_err() {
                        let fallback = format!("sources[{i}]");
                        let label = name.as_deref().unwrap_or(&fallback);
                        errors.push(format!("source '{label}' has invalid URL: {url}"));
                    }
                }
                SourceConfig::GitHubTrending { since, .. } => {
                    if !["daily", "weekly", "monthly"].contains(&since.as_str()) {
                        errors.push(format!(
                            "sources[{i}].since '{since}' is not valid (expected: daily, weekly, monthly)"
                        ));
                    }
                }
                SourceConfig::HackerNews { limit } => {
                    if *limit == 0 {
                        errors.push(format!("sources[{i}] hackernews limit must be > 0"));
                    }
                }
                SourceConfig::Reddit { subreddits, limit } => {
                    if subreddits.is_empty() {
                        errors.push(format!("sources[{i}] reddit must have at least one subreddit"));
                    }
                    if *limit == 0 {
                        errors.push(format!("sources[{i}] reddit limit must be > 0"));
                    }
                }
            }
        }

        // 2. Reporter format
        if !VALID_FORMATS.contains(&self.reporter.format.as_str()) {
            errors.push(format!(
                "reporter.format '{}' is not valid (expected one of: {})",
                self.reporter.format,
                VALID_FORMATS.join(", "),
            ));
        }

        // 3. Analyzer timeout
        if self.analyzer.timeout_secs == 0 {
            errors.push("analyzer.timeout_secs must be > 0".to_string());
        } else if self.analyzer.timeout_secs > MAX_TIMEOUT_SECS {
            errors.push(format!(
                "analyzer.timeout_secs {} exceeds maximum of {MAX_TIMEOUT_SECS}",
                self.analyzer.timeout_secs,
            ));
        }

        // 4. Log level
        if !VALID_LOG_LEVELS.contains(&self.general.log_level.as_str()) {
            errors.push(format!(
                "general.log_level '{}' is not valid (expected one of: {})",
                self.general.log_level,
                VALID_LOG_LEVELS.join(", "),
            ));
        }

        // 5. Batch size
        if self.general.backfill_batch_size == 0 {
            errors.push("general.backfill_batch_size must be > 0".to_string());
        } else if self.general.backfill_batch_size > MAX_BATCH_SIZE {
            errors.push(format!(
                "general.backfill_batch_size {} exceeds maximum of {MAX_BATCH_SIZE}",
                self.general.backfill_batch_size,
            ));
        }

        // 6. GitHub token validation
        if let Some(ref token) = self.general.github_token {
            let trimmed = token.trim();
            if trimmed.is_empty() {
                errors.push("general.github_token is set but empty".to_string());
            } else if trimmed.len() <= 10 {
                errors.push(format!(
                    "general.github_token looks too short ({} chars) — \
                     GitHub tokens are typically 40+ characters",
                    trimmed.len(),
                ));
            }
            const PLACEHOLDERS: &[&str] = &[
                "xxx",
                "your-token-here",
                "your_token_here",
                "TOKEN",
                "GITHUB_TOKEN",
                "ghp_xxxx",
                "replace-me",
            ];
            if PLACEHOLDERS
                .iter()
                .any(|p| trimmed.eq_ignore_ascii_case(p))
            {
                tracing::warn!(
                    "general.github_token looks like a placeholder ('{}') — \
                     API calls will likely fail",
                    trimmed,
                );
            }
        }

        // 7. Repoforge path soft warning
        if let Some(ref path) = self.analyzer.repoforge_path
            && !path.exists()
        {
            tracing::warn!(
                "analyzer.repoforge_path '{}' does not exist — \
                 the binary may need to be installed",
                path.display(),
            );
        }

        if errors.is_empty() {
            Ok(())
        } else {
            let numbered: Vec<String> = errors
                .iter()
                .enumerate()
                .map(|(i, e)| format!("{}. {e}", i + 1))
                .collect();
            Err(PipelineError::Config(format!(
                "Config validation failed:\n{}",
                numbered.join("\n"),
            )))
        }
    }
}

/// An RSS/Atom feed source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedConfig {
    pub url: String,
    #[serde(default)]
    pub name: Option<String>,
    /// Max entries to process from this feed (newest first). `None` = no limit.
    #[serde(default)]
    pub limit: Option<usize>,
}

/// A source configuration with type discriminator.
///
/// Uses serde's internally-tagged representation so TOML looks like:
/// ```toml
/// [[sources]]
/// type = "rss"
/// url = "https://example.com/feed.xml"
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum SourceConfig {
    /// RSS/Atom feed source.
    #[serde(rename = "rss")]
    Rss {
        url: String,
        #[serde(default)]
        name: Option<String>,
    },
    /// GitHub Trending page scraper.
    #[serde(rename = "github_trending")]
    GitHubTrending {
        #[serde(default)]
        language: Option<String>,
        /// Trending period: "daily", "weekly", or "monthly".
        #[serde(default = "default_trending_since")]
        since: String,
    },
    /// HackerNews "Show HN" stories with GitHub links.
    #[serde(rename = "hackernews")]
    HackerNews {
        /// Maximum number of stories to fetch (default: 30).
        #[serde(default = "default_hn_limit")]
        limit: usize,
    },
    /// Reddit subreddit posts with GitHub links.
    #[serde(rename = "reddit")]
    Reddit {
        subreddits: Vec<String>,
        /// Maximum posts per subreddit (default: 25).
        #[serde(default = "default_reddit_limit")]
        limit: usize,
    },
}

fn default_trending_since() -> String {
    "daily".into()
}

const fn default_hn_limit() -> usize {
    30
}

const fn default_reddit_limit() -> usize {
    25
}

/// Filtering criteria for discovered repos.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterConfig {
    #[serde(default = "default_min_stars")]
    pub min_stars: u64,
    #[serde(default)]
    pub languages: Vec<String>,
    #[serde(default)]
    pub topics: Vec<String>,
    #[serde(default)]
    pub exclude_forks: bool,
    #[serde(default)]
    pub exclude_archived: bool,
}

impl Default for FilterConfig {
    fn default() -> Self {
        Self {
            min_stars: default_min_stars(),
            languages: Vec::new(),
            topics: Vec::new(),
            exclude_forks: true,
            exclude_archived: true,
        }
    }
}

const fn default_min_stars() -> u64 {
    10
}

/// Analyzer settings (repoforge + LLM).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalyzerConfig {
    #[serde(default)]
    pub repoforge_path: Option<PathBuf>,
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    #[serde(default)]
    pub llm_model: Option<String>,
    /// Loaded from `REPO_RADAR_LLM_API_KEY` env var at runtime.
    #[serde(skip)]
    pub llm_api_key: Option<String>,
}

impl Default for AnalyzerConfig {
    fn default() -> Self {
        Self {
            repoforge_path: None,
            timeout_secs: default_timeout_secs(),
            llm_model: None,
            llm_api_key: None,
        }
    }
}

const fn default_timeout_secs() -> u64 {
    60
}

/// Cross-reference settings.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CrossRefConfig {
    /// Paths to repoforge JSON exports of the user's own repos.
    #[serde(default)]
    pub own_repos: Vec<PathBuf>,
    /// Loaded from `REPO_RADAR_GITHUB_USERNAME` env var at runtime.
    #[serde(skip)]
    pub github_username: Option<String>,
}

/// Reporter output settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReporterConfig {
    #[serde(default = "default_output_dir")]
    pub output_dir: PathBuf,
    #[serde(default = "default_format")]
    pub format: String,
}

impl Default for ReporterConfig {
    fn default() -> Self {
        Self {
            output_dir: default_output_dir(),
            format: default_format(),
        }
    }
}

fn default_output_dir() -> PathBuf {
    PathBuf::from("./output")
}

fn default_format() -> String {
    "markdown".into()
}

/// General runtime settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralConfig {
    #[serde(default = "default_data_dir")]
    pub data_dir: PathBuf,
    #[serde(default = "default_log_level")]
    pub log_level: String,
    #[serde(default = "default_backfill_batch_size")]
    pub backfill_batch_size: usize,
    /// Loaded from `REPO_RADAR_GITHUB_TOKEN` env var at runtime.
    #[serde(skip)]
    pub github_token: Option<String>,
    /// Optional bearer token for dashboard authentication.
    /// Loaded from `REPO_RADAR_DASHBOARD_TOKEN` env var at runtime.
    #[serde(skip)]
    pub dashboard_token: Option<String>,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            data_dir: default_data_dir(),
            log_level: default_log_level(),
            backfill_batch_size: default_backfill_batch_size(),
            github_token: None,
            dashboard_token: None,
        }
    }
}

fn default_data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("repo-radar")
}

fn default_log_level() -> String {
    "info".into()
}

const fn default_backfill_batch_size() -> usize {
    50
}

/// Cache settings for GitHub API response caching.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheConfig {
    /// Time-to-live for cached entries in seconds (default: 24 hours).
    #[serde(default = "default_cache_ttl_secs")]
    pub ttl_secs: u64,
    /// Directory for cache files. Defaults to `data_dir/cache`.
    #[serde(default)]
    pub cache_dir: Option<PathBuf>,
    /// Log a warning when remaining API calls drop below this threshold.
    #[serde(default = "default_rate_limit_threshold")]
    pub rate_limit_threshold: u32,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            ttl_secs: default_cache_ttl_secs(),
            cache_dir: None,
            rate_limit_threshold: default_rate_limit_threshold(),
        }
    }
}

const fn default_cache_ttl_secs() -> u64 {
    86_400 // 24 hours
}

const fn default_rate_limit_threshold() -> u32 {
    100
}

/// Returns the default XDG config path for repo-radar.
#[must_use]
pub fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from(".config"))
        .join("repo-radar")
        .join("config.toml")
}

/// Load configuration from a TOML file, with env var overlays for secrets.
///
/// If `path` is `None`, resolves to the XDG default. If the file does not
/// exist, returns `AppConfig::default()`.
///
/// # Errors
///
/// Returns `PipelineError::Config` if the file exists but cannot be read or parsed.
pub fn load_config(path: Option<&Path>) -> Result<AppConfig, PipelineError> {
    let resolved = path.map_or_else(config_path, Path::to_path_buf);

    let mut config = if resolved.exists() {
        let content = std::fs::read_to_string(&resolved)
            .map_err(|e| PipelineError::Config(format!("reading {}: {e}", resolved.display())))?;
        toml::from_str::<AppConfig>(&content)
            .map_err(|e| PipelineError::Config(format!("parsing {}: {e}", resolved.display())))?
    } else {
        AppConfig::default()
    };

    // Env var overlays for secrets
    if let Ok(token) = std::env::var("REPO_RADAR_GITHUB_TOKEN") {
        config.general.github_token = Some(token);
    } else if config.general.github_token.is_none() {
        // Fallback: try `gh auth token` if gh CLI is available and authenticated
        if let Ok(output) = std::process::Command::new("gh")
            .args(["auth", "token"])
            .output()
        {
            if output.status.success() {
                let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !token.is_empty() {
                    tracing::debug!("GitHub token resolved from gh CLI");
                    config.general.github_token = Some(token);
                }
            }
        }
    }
    if let Ok(key) = std::env::var("REPO_RADAR_LLM_API_KEY") {
        config.analyzer.llm_api_key = Some(key);
    }
    if let Ok(username) = std::env::var("REPO_RADAR_GITHUB_USERNAME") {
        config.crossref.github_username = Some(username);
    } else if config.crossref.github_username.is_none() {
        // Fallback: resolve username from `gh api user`
        if let Ok(output) = std::process::Command::new("gh")
            .args(["api", "user", "--jq", ".login"])
            .output()
        {
            if output.status.success() {
                let username = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !username.is_empty() {
                    tracing::debug!("GitHub username resolved from gh CLI: {username}");
                    config.crossref.github_username = Some(username);
                }
            }
        }
    }
    if let Ok(dashboard_token) = std::env::var("REPO_RADAR_DASHBOARD_TOKEN") {
        config.general.dashboard_token = Some(dashboard_token);
    }

    config.validate()?;

    Ok(config)
}

/// Returns a commented TOML template for a default config file.
#[must_use]
pub fn default_config() -> String {
    r#"# repo-radar configuration
# See: https://github.com/JNZader/repo-radar

# RSS/Atom feeds to monitor for new repos
[[feeds]]
url = "https://rsshub.app/github/trending/daily"
name = "GitHub Trending (Daily)"

# [[feeds]]
# url = "https://your-other-feed.example.com/rss"
# name = "Custom Feed"

# ── Additional sources (optional) ──────────────────────────────
# Each [[sources]] entry needs a `type` field.

# [[sources]]
# type = "github_trending"
# language = "rust"     # optional language filter
# since = "daily"       # daily | weekly | monthly

# [[sources]]
# type = "hackernews"
# limit = 30            # max Show HN stories to fetch

# [[sources]]
# type = "reddit"
# subreddits = ["rust", "programming"]
# limit = 25            # max posts per subreddit

# Filtering criteria
[filter]
min_stars = 10
languages = []          # e.g. ["Rust", "TypeScript"]
topics = []             # e.g. ["cli", "web"]
exclude_forks = true
exclude_archived = true

# Analyzer settings
[analyzer]
# repoforge_path = "/path/to/repoforge"
timeout_secs = 60
# llm_model = "gpt-4o-mini"
# API key loaded from REPO_RADAR_LLM_API_KEY env var

# Cross-reference with your own repos
[crossref]
own_repos = []          # paths to repoforge JSON exports
# GitHub username loaded from REPO_RADAR_GITHUB_USERNAME env var

# Reporter output
[reporter]
output_dir = "./output"
format = "markdown"     # markdown | json

# General settings
[general]
# data_dir auto-resolves to XDG data dir
log_level = "info"
backfill_batch_size = 50
# GitHub token loaded from REPO_RADAR_GITHUB_TOKEN env var

# API response cache settings
[cache]
ttl_secs = 86400            # 24 hours — how long cached metadata stays fresh
# cache_dir = "./cache"     # defaults to data_dir/cache
rate_limit_threshold = 100  # warn when remaining API calls drop below this
"#
    .to_string()
}

/// Write the default config template to the given path, creating parent dirs.
///
/// # Errors
///
/// Returns `PipelineError::Config` if directories or file cannot be created.
pub fn write_default_config(path: &Path) -> Result<(), PipelineError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| PipelineError::Config(format!("creating {}: {e}", parent.display())))?;
    }
    std::fs::write(path, default_config())
        .map_err(|e| PipelineError::Config(format!("writing {}: {e}", path.display())))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    #[test]
    fn default_config_toml_parses() {
        let toml_str = default_config();
        let config: AppConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(config.feeds.len(), 1);
        assert_eq!(config.filter.min_stars, 10);
        assert!(config.filter.exclude_forks);
        assert!(config.filter.exclude_archived);
        assert_eq!(config.analyzer.timeout_secs, 60);
        assert_eq!(config.reporter.format, "markdown");
    }

    #[test]
    fn minimal_config_only_required_fields() {
        let toml_str = "";
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert!(config.feeds.is_empty());
        assert_eq!(config.filter.min_stars, default_min_stars());
        assert!(config.crossref.own_repos.is_empty());
    }

    #[test]
    fn config_with_all_fields() {
        let toml_str = r#"
[[feeds]]
url = "https://feed1.example.com/rss"
name = "Feed One"

[[feeds]]
url = "https://feed2.example.com/rss"

[filter]
min_stars = 100
languages = ["Rust", "Go"]
topics = ["cli"]
exclude_forks = false
exclude_archived = false

[analyzer]
repoforge_path = "/usr/bin/repoforge"
timeout_secs = 120
llm_model = "gpt-4o"

[crossref]
own_repos = ["/data/repos.json"]

[reporter]
output_dir = "/tmp/reports"
format = "json"

[general]
data_dir = "/tmp/data"
log_level = "debug"
backfill_batch_size = 100
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.feeds.len(), 2);
        assert_eq!(config.feeds[0].name.as_deref(), Some("Feed One"));
        assert!(config.feeds[1].name.is_none());
        assert_eq!(config.filter.min_stars, 100);
        assert_eq!(config.filter.languages, vec!["Rust", "Go"]);
        assert!(!config.filter.exclude_forks);
        assert_eq!(config.analyzer.timeout_secs, 120);
        assert_eq!(config.analyzer.llm_model.as_deref(), Some("gpt-4o"));
        assert_eq!(config.crossref.own_repos.len(), 1);
        assert_eq!(config.reporter.format, "json");
        assert_eq!(config.general.log_level, "debug");
        assert_eq!(config.general.backfill_batch_size, 100);
    }

    #[test]
    fn load_config_nonexistent_path_returns_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nonexistent.toml");
        let config = load_config(Some(&path)).unwrap();
        assert!(config.feeds.is_empty());
        assert_eq!(config.filter.min_stars, default_min_stars());
    }

    #[test]
    fn load_config_from_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
[[feeds]]
url = "https://example.com/rss"

[filter]
min_stars = 50
"#,
        )
        .unwrap();

        let config = load_config(Some(&path)).unwrap();
        assert_eq!(config.feeds.len(), 1);
        assert_eq!(config.filter.min_stars, 50);
    }

    #[test]
    fn load_config_invalid_toml_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.toml");
        std::fs::write(&path, "not valid toml {{{{").unwrap();

        let result = load_config(Some(&path));
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("parsing"));
    }

    #[test]
    #[serial]
    fn env_var_overlay_github_token() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "").unwrap();

        // SAFETY: test runs single-threaded; no other threads read this env var concurrently.
        unsafe { std::env::set_var("REPO_RADAR_GITHUB_TOKEN", "test-token-123") };
        let config = load_config(Some(&path)).unwrap();
        unsafe { std::env::remove_var("REPO_RADAR_GITHUB_TOKEN") };

        assert_eq!(config.general.github_token.as_deref(), Some("test-token-123"));
    }

    #[test]
    #[serial]
    fn env_var_overlay_llm_api_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "").unwrap();

        // SAFETY: test runs single-threaded; no other threads read this env var concurrently.
        unsafe { std::env::set_var("REPO_RADAR_LLM_API_KEY", "sk-test-key") };
        let config = load_config(Some(&path)).unwrap();
        unsafe { std::env::remove_var("REPO_RADAR_LLM_API_KEY") };

        assert_eq!(config.analyzer.llm_api_key.as_deref(), Some("sk-test-key"));
    }

    #[test]
    #[serial]
    fn env_var_overlay_github_username() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "").unwrap();

        // SAFETY: test runs single-threaded; no other threads read this env var concurrently.
        unsafe { std::env::set_var("REPO_RADAR_GITHUB_USERNAME", "octocat") };
        let config = load_config(Some(&path)).unwrap();
        unsafe { std::env::remove_var("REPO_RADAR_GITHUB_USERNAME") };

        assert_eq!(
            config.crossref.github_username.as_deref(),
            Some("octocat"),
        );
    }

    #[test]
    fn crossref_config_serde_round_trip() {
        let toml_str = r#"
[crossref]
own_repos = ["/data/my-repos.json", "/data/other.json"]
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.crossref.own_repos.len(), 2);
        // github_username is #[serde(skip)], so it must be None after deserialization
        assert!(config.crossref.github_username.is_none());

        // Re-serialize and deserialize to confirm round-trip stability
        let serialized = toml::to_string(&config).unwrap();
        let roundtrip: AppConfig = toml::from_str(&serialized).unwrap();
        assert_eq!(roundtrip.crossref.own_repos.len(), 2);
        assert!(roundtrip.crossref.github_username.is_none());
    }

    #[test]
    fn write_default_config_creates_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("config.toml");
        write_default_config(&path).unwrap();

        assert!(path.exists());
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("repo-radar configuration"));
    }

    #[test]
    fn config_empty_feeds_is_valid() {
        let toml_str = r#"
[filter]
min_stars = 5
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert!(config.feeds.is_empty());
    }

    #[test]
    fn config_filter_defaults_are_sensible() {
        let config = FilterConfig::default();
        assert_eq!(config.min_stars, 10);
        assert!(config.exclude_forks);
        assert!(config.exclude_archived);
    }

    #[test]
    fn config_reporter_default_format_is_markdown() {
        let config = ReporterConfig::default();
        assert_eq!(config.format, "markdown");
    }

    #[test]
    fn config_reporter_default_output_dir() {
        let config = ReporterConfig::default();
        assert_eq!(config.output_dir, PathBuf::from("./output"));
    }

    #[test]
    fn config_analyzer_default_timeout() {
        let toml_str = "[analyzer]\n";
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.analyzer.timeout_secs, 60);
    }

    // ── Validation tests ─────────────────────────────────────────────

    #[test]
    fn validate_accepts_valid_config() {
        let config = AppConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_rejects_invalid_feed_url() {
        let mut config = AppConfig::default();
        config.feeds.push(FeedConfig {
            url: "not a url".into(),
            name: Some("Bad Feed".into()),
            limit: None,
        });
        let err = config.validate().unwrap_err().to_string();
        assert!(err.contains("invalid URL"), "error was: {err}");
        assert!(err.contains("Bad Feed"), "error was: {err}");
    }

    #[test]
    fn validate_rejects_unknown_reporter_format() {
        let mut config = AppConfig::default();
        config.reporter.format = "pdf".into();
        let err = config.validate().unwrap_err().to_string();
        assert!(err.contains("reporter.format"), "error was: {err}");
        assert!(err.contains("pdf"), "error was: {err}");
    }

    #[test]
    fn validate_rejects_zero_timeout() {
        let mut config = AppConfig::default();
        config.analyzer.timeout_secs = 0;
        let err = config.validate().unwrap_err().to_string();
        assert!(err.contains("timeout_secs must be > 0"), "error was: {err}");
    }

    #[test]
    fn validate_rejects_excessive_timeout() {
        let mut config = AppConfig::default();
        config.analyzer.timeout_secs = 9999;
        let err = config.validate().unwrap_err().to_string();
        assert!(err.contains("exceeds maximum"), "error was: {err}");
    }

    #[test]
    fn validate_rejects_invalid_log_level() {
        let mut config = AppConfig::default();
        config.general.log_level = "bananas".into();
        let err = config.validate().unwrap_err().to_string();
        assert!(err.contains("log_level"), "error was: {err}");
        assert!(err.contains("bananas"), "error was: {err}");
    }

    #[test]
    fn validate_rejects_zero_batch_size() {
        let mut config = AppConfig::default();
        config.general.backfill_batch_size = 0;
        let err = config.validate().unwrap_err().to_string();
        assert!(
            err.contains("backfill_batch_size must be > 0"),
            "error was: {err}",
        );
    }

    #[test]
    fn validate_collects_multiple_errors() {
        let mut config = AppConfig::default();
        config.reporter.format = "xml".into();
        config.analyzer.timeout_secs = 0;
        config.general.log_level = "bananas".into();
        let err = config.validate().unwrap_err().to_string();
        // Must contain numbered errors — at least 3
        assert!(err.contains("1."), "error was: {err}");
        assert!(err.contains("2."), "error was: {err}");
        assert!(err.contains("3."), "error was: {err}");
    }

    #[test]
    fn validate_accepts_all_reporter_formats() {
        for format in &["markdown", "json", "console"] {
            let mut config = AppConfig::default();
            config.reporter.format = (*format).to_string();
            assert!(config.validate().is_ok(), "format '{format}' should be valid");
        }
    }

    #[test]
    fn validate_accepts_empty_feeds() {
        let config = AppConfig::default();
        assert!(config.feeds.is_empty());
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_rejects_empty_github_token() {
        let mut config = AppConfig::default();
        config.general.github_token = Some("".into());
        let err = config.validate().unwrap_err().to_string();
        assert!(err.contains("github_token is set but empty"), "error was: {err}");
    }

    #[test]
    fn validate_rejects_short_github_token() {
        let mut config = AppConfig::default();
        config.general.github_token = Some("abc".into());
        let err = config.validate().unwrap_err().to_string();
        assert!(err.contains("too short"), "error was: {err}");
    }

    #[test]
    fn validate_accepts_valid_github_token() {
        let mut config = AppConfig::default();
        config.general.github_token = Some("ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghij".into());
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_accepts_none_github_token() {
        let config = AppConfig::default();
        assert!(config.general.github_token.is_none());
        assert!(config.validate().is_ok());
    }

    #[test]
    #[serial]
    fn env_vars_dont_persist_to_saved_config() {
        let dir = tempfile::tempdir().unwrap();
        let src_path = dir.path().join("source.toml");
        let dst_path = dir.path().join("saved.toml");
        std::fs::write(&src_path, "").unwrap();

        // SAFETY: test runs single-threaded; no other threads read these env vars concurrently.
        unsafe { std::env::set_var("REPO_RADAR_GITHUB_TOKEN", "secret-token") };
        unsafe { std::env::set_var("REPO_RADAR_LLM_API_KEY", "secret-key") };
        unsafe { std::env::set_var("REPO_RADAR_GITHUB_USERNAME", "secret-user") };

        let config = load_config(Some(&src_path)).unwrap();
        assert_eq!(config.general.github_token.as_deref(), Some("secret-token"));

        // Serialize to TOML and write — skip fields should NOT appear
        let serialized = toml::to_string(&config).unwrap();
        std::fs::write(&dst_path, &serialized).unwrap();

        unsafe { std::env::remove_var("REPO_RADAR_GITHUB_TOKEN") };
        unsafe { std::env::remove_var("REPO_RADAR_LLM_API_KEY") };
        unsafe { std::env::remove_var("REPO_RADAR_GITHUB_USERNAME") };

        // Reload from saved file WITHOUT env vars.
        // Token/username may be resolved from `gh auth token` fallback — that's fine.
        // What matters is that the *env var secret values* are NOT present.
        let reloaded = load_config(Some(&dst_path)).unwrap();
        assert_ne!(reloaded.general.github_token.as_deref(), Some("secret-token"));
        assert!(reloaded.analyzer.llm_api_key.is_none());
        assert_ne!(reloaded.crossref.github_username.as_deref(), Some("secret-user"));

        // Also verify the raw TOML doesn't contain the secrets
        let raw = std::fs::read_to_string(&dst_path).unwrap();
        assert!(!raw.contains("secret-token"));
        assert!(!raw.contains("secret-key"));
        assert!(!raw.contains("secret-user"));
    }

    // ── Source config tests ─────────────────────────────────────────

    #[test]
    fn source_config_rss_parses() {
        let toml_str = r#"
[[sources]]
type = "rss"
url = "https://example.com/feed.xml"
name = "My Feed"
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.sources.len(), 1);
        match &config.sources[0] {
            SourceConfig::Rss { url, name } => {
                assert_eq!(url, "https://example.com/feed.xml");
                assert_eq!(name.as_deref(), Some("My Feed"));
            }
            other => panic!("expected Rss, got {other:?}"),
        }
    }

    #[test]
    fn source_config_github_trending_parses() {
        let toml_str = r#"
[[sources]]
type = "github_trending"
language = "rust"
since = "weekly"
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.sources.len(), 1);
        match &config.sources[0] {
            SourceConfig::GitHubTrending { language, since } => {
                assert_eq!(language.as_deref(), Some("rust"));
                assert_eq!(since, "weekly");
            }
            other => panic!("expected GitHubTrending, got {other:?}"),
        }
    }

    #[test]
    fn source_config_github_trending_defaults() {
        let toml_str = r#"
[[sources]]
type = "github_trending"
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        match &config.sources[0] {
            SourceConfig::GitHubTrending { language, since } => {
                assert!(language.is_none());
                assert_eq!(since, "daily");
            }
            other => panic!("expected GitHubTrending, got {other:?}"),
        }
    }

    #[test]
    fn source_config_hackernews_parses() {
        let toml_str = r#"
[[sources]]
type = "hackernews"
limit = 50
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        match &config.sources[0] {
            SourceConfig::HackerNews { limit } => {
                assert_eq!(*limit, 50);
            }
            other => panic!("expected HackerNews, got {other:?}"),
        }
    }

    #[test]
    fn source_config_hackernews_defaults() {
        let toml_str = r#"
[[sources]]
type = "hackernews"
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        match &config.sources[0] {
            SourceConfig::HackerNews { limit } => {
                assert_eq!(*limit, 30);
            }
            other => panic!("expected HackerNews, got {other:?}"),
        }
    }

    #[test]
    fn source_config_reddit_parses() {
        let toml_str = r#"
[[sources]]
type = "reddit"
subreddits = ["rust", "programming"]
limit = 10
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        match &config.sources[0] {
            SourceConfig::Reddit { subreddits, limit } => {
                assert_eq!(subreddits, &["rust", "programming"]);
                assert_eq!(*limit, 10);
            }
            other => panic!("expected Reddit, got {other:?}"),
        }
    }

    #[test]
    fn source_config_mixed_sources() {
        let toml_str = r#"
[[sources]]
type = "rss"
url = "https://example.com/feed.xml"

[[sources]]
type = "github_trending"

[[sources]]
type = "hackernews"

[[sources]]
type = "reddit"
subreddits = ["rust"]
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.sources.len(), 4);
        assert!(matches!(&config.sources[0], SourceConfig::Rss { .. }));
        assert!(matches!(&config.sources[1], SourceConfig::GitHubTrending { .. }));
        assert!(matches!(&config.sources[2], SourceConfig::HackerNews { .. }));
        assert!(matches!(&config.sources[3], SourceConfig::Reddit { .. }));
    }

    #[test]
    fn source_config_backward_compat_feeds_only() {
        let toml_str = r#"
[[feeds]]
url = "https://example.com/feed.xml"
name = "Legacy Feed"
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.feeds.len(), 1);
        assert!(config.sources.is_empty());
    }

    #[test]
    fn source_config_feeds_and_sources_coexist() {
        let toml_str = r#"
[[feeds]]
url = "https://example.com/feed.xml"

[[sources]]
type = "github_trending"
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.feeds.len(), 1);
        assert_eq!(config.sources.len(), 1);
    }

    #[test]
    fn validate_rejects_invalid_trending_since() {
        let mut config = AppConfig::default();
        config.sources.push(SourceConfig::GitHubTrending {
            language: None,
            since: "yearly".into(),
        });
        let err = config.validate().unwrap_err().to_string();
        assert!(err.contains("yearly"), "error was: {err}");
    }

    #[test]
    fn validate_rejects_zero_hn_limit() {
        let mut config = AppConfig::default();
        config.sources.push(SourceConfig::HackerNews { limit: 0 });
        let err = config.validate().unwrap_err().to_string();
        assert!(err.contains("limit must be > 0"), "error was: {err}");
    }

    #[test]
    fn validate_rejects_empty_subreddits() {
        let mut config = AppConfig::default();
        config.sources.push(SourceConfig::Reddit {
            subreddits: vec![],
            limit: 25,
        });
        let err = config.validate().unwrap_err().to_string();
        assert!(err.contains("at least one subreddit"), "error was: {err}");
    }

    #[test]
    fn validate_rejects_zero_reddit_limit() {
        let mut config = AppConfig::default();
        config.sources.push(SourceConfig::Reddit {
            subreddits: vec!["rust".into()],
            limit: 0,
        });
        let err = config.validate().unwrap_err().to_string();
        assert!(err.contains("reddit limit must be > 0"), "error was: {err}");
    }

    #[test]
    fn validate_accepts_valid_sources() {
        let mut config = AppConfig::default();
        config.sources.push(SourceConfig::Rss {
            url: "https://example.com/feed.xml".into(),
            name: None,
        });
        config.sources.push(SourceConfig::GitHubTrending {
            language: Some("rust".into()),
            since: "daily".into(),
        });
        config.sources.push(SourceConfig::HackerNews { limit: 30 });
        config.sources.push(SourceConfig::Reddit {
            subreddits: vec!["rust".into()],
            limit: 25,
        });
        assert!(config.validate().is_ok());
    }
}
