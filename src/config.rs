use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::infra::error::PipelineError;

/// Top-level application configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub feeds: Vec<FeedConfig>,
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
}

/// An RSS/Atom feed source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedConfig {
    pub url: String,
    #[serde(default)]
    pub name: Option<String>,
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
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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

const fn default_timeout_secs() -> u64 {
    60
}

/// Cross-reference settings.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CrossRefConfig {
    /// Paths to repoforge JSON exports of the user's own repos.
    #[serde(default)]
    pub own_repos: Vec<PathBuf>,
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
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            data_dir: default_data_dir(),
            log_level: default_log_level(),
            backfill_batch_size: default_backfill_batch_size(),
            github_token: None,
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
    }
    if let Ok(key) = std::env::var("REPO_RADAR_LLM_API_KEY") {
        config.analyzer.llm_api_key = Some(key);
    }

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
