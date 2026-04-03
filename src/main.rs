use clap::Parser;
use miette::{IntoDiagnostic, Result};
use owo_colors::OwoColorize;
use tracing::info;
use tracing_subscriber::EnvFilter;

use repo_radar::adapters::analyzer::{AnalyzerAdapter, NoopAnalyzer, RepoforgeAnalyzer};
use repo_radar::adapters::categorizer::{CategorizerAdapter, KeywordCategorizer};
use repo_radar::adapters::crossref::{CrossRefAdapter, NoopCrossRef};
use repo_radar::adapters::crossref::github_crossref::GitHubCrossRef;
use repo_radar::adapters::filter::{FilterAdapter, GitHubMetadataFilter, NoopFilter};
use repo_radar::adapters::reporter::{
    ConsoleReporter, JsonReporter, MarkdownReporter, ReporterAdapter,
};
use repo_radar::adapters::source::{
    GitHubTrendingSource, HackerNewsSource, MultiSource, NoopSource, RedditSource, RssSource,
    SourceAdapter,
};
use repo_radar::adapters::web::{self, AppState};
use repo_radar::cli::{Cli, Command, ConfigAction};
use repo_radar::config::{config_path, load_config, write_default_config, SourceConfig};
use repo_radar::infra::cache::RepoCache;
use repo_radar::infra::seen::SeenStore;
use repo_radar::pipeline::Pipeline;

fn init_tracing(verbose: u8) {
    let filter = match verbose {
        0 => "info",
        1 => "debug",
        _ => "trace",
    };

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(filter)),
        )
        .without_time()
        .init();
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    init_tracing(cli.verbose);

    match cli.command {
        Command::Config { ref action } => handle_config(action, cli.config.as_deref())?,
        Command::Scan {
            dry_run,
            stage: _,
            backfill: _,
        } => handle_scan(cli.config.as_deref(), dry_run).await?,
        Command::Serve { port, ref host } => {
            handle_serve(cli.config.as_deref(), port, host).await?
        }
        Command::Report {
            ref format,
            ref output,
        } => {
            let config = load_config(cli.config.as_deref()).into_diagnostic()?;
            let output_dir = output
                .clone()
                .unwrap_or_else(|| config.reporter.output_dir.clone());

            let _reporter = match format.as_str() {
                "json" => ReporterAdapter::Json(JsonReporter::new(output_dir)),
                "console" => ReporterAdapter::Console(ConsoleReporter::new()),
                _ => ReporterAdapter::Markdown(MarkdownReporter::new(output_dir)),
            };

            eprintln!(
                "{} Report generation requires cached scan results, which are not yet available. \
                 Run `repo-radar scan` first, then re-run this command once caching is implemented.",
                "TODO:".yellow().bold()
            );
        }
    }

    Ok(())
}

fn handle_config(action: &ConfigAction, config_override: Option<&std::path::Path>) -> Result<()> {
    match action {
        ConfigAction::Init => {
            let path = config_override.map(|p| p.to_path_buf()).unwrap_or_else(config_path);
            if path.exists() {
                eprintln!(
                    "{} Config already exists at {}",
                    "Note:".cyan().bold(),
                    path.display()
                );
                return Ok(());
            }
            write_default_config(&path).into_diagnostic()?;
            println!(
                "{} Created default config at {}",
                "Done:".green().bold(),
                path.display()
            );
        }
        ConfigAction::Show => {
            let config = load_config(config_override).into_diagnostic()?;
            let toml_str =
                toml::to_string_pretty(&config).into_diagnostic()?;
            println!("{toml_str}");
        }
    }
    Ok(())
}

async fn handle_scan(config_path_override: Option<&std::path::Path>, dry_run: bool) -> Result<()> {
    // Check if config exists; if not, show first-run message (REQ-10)
    let resolved_path =
        config_path_override.map_or_else(config_path, std::path::Path::to_path_buf);

    if !resolved_path.exists() {
        let xdg_path = config_path();
        println!(
            "\n{} No config file found.\n\n  Run {} to create one at:\n  {}\n",
            "Welcome to repo-radar!".green().bold(),
            "repo-radar config init".cyan().bold(),
            xdg_path.display()
        );
        return Ok(());
    }

    let config = load_config(config_path_override).into_diagnostic()?;
    info!("config loaded");

    if dry_run {
        println!(
            "{} Dry-run mode — showing resolved config:\n",
            "Dry run:".yellow().bold()
        );
        let output = toml::to_string_pretty(&config).into_diagnostic()?;
        println!("{output}");
        return Ok(());
    }

    // Build pipeline — real adapters for source/filter, Noop for rest (Phase 3-4)
    let seen_path = config.general.data_dir.join("seen.json");
    let seen = SeenStore::load(&seen_path).into_diagnostic()?;

    // Build HTTP client shared across sources
    let http_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .into_diagnostic()?;

    // Build sources from config.sources + backward-compat config.feeds
    let source = build_source(&config, &http_client);

    // Filter: GitHub metadata if token available, otherwise Noop
    let filter = if config.general.github_token.is_some() || !config.feeds.is_empty() {
        let cache_dir = config
            .cache
            .cache_dir
            .clone()
            .unwrap_or_else(|| config.general.data_dir.join("cache"));
        let cache_path = cache_dir.join("repo_metadata.json");
        let cache_ttl = std::time::Duration::from_secs(config.cache.ttl_secs);
        let repo_cache = RepoCache::load(&cache_path, cache_ttl).into_diagnostic()?;

        let gh_filter = GitHubMetadataFilter::new(
            config.filter.clone(),
            config.general.github_token.as_deref(),
            Some(repo_cache),
            config.cache.rate_limit_threshold,
        )
        .into_diagnostic()?;
        FilterAdapter::GitHubMetadata(Box::new(gh_filter))
    } else {
        FilterAdapter::Noop(NoopFilter)
    };

    // Analyzer: RepoForge if path configured, otherwise Noop
    let analyzer = if let Some(ref repoforge_path) = config.analyzer.repoforge_path {
        AnalyzerAdapter::Repoforge(RepoforgeAnalyzer::new(
            repoforge_path.clone(),
            config.analyzer.timeout_secs,
        ))
    } else {
        AnalyzerAdapter::Noop(NoopAnalyzer)
    };

    // CrossRef: GitHub if username configured, otherwise Noop
    let crossref = if let Some(ref username) = config.crossref.github_username {
        let gh_crossref = GitHubCrossRef::new(
            username.clone(),
            config.general.github_token.as_deref(),
        )
        .into_diagnostic()?;
        CrossRefAdapter::GitHub(Box::new(gh_crossref))
    } else {
        CrossRefAdapter::Noop(NoopCrossRef)
    };
    let reporter = match config.reporter.format.as_str() {
        "json" => ReporterAdapter::Json(JsonReporter::new(config.reporter.output_dir.clone())),
        "console" => ReporterAdapter::Console(ConsoleReporter::new()),
        // "markdown" and any unrecognized format default to Markdown
        _ => ReporterAdapter::Markdown(MarkdownReporter::new(config.reporter.output_dir.clone())),
    };

    // Categorizer: always use keyword-based categorizer
    let categorizer = CategorizerAdapter::Keyword(KeywordCategorizer::new());

    let mut pipeline = Pipeline::new(source, filter, categorizer, analyzer, crossref, reporter, seen, None);
    let report = pipeline.run().await.into_diagnostic()?;

    println!("\n{}\n{report}", "Scan complete:".green().bold());

    Ok(())
}

async fn handle_serve(
    config_path_override: Option<&std::path::Path>,
    port: u16,
    host: &str,
) -> Result<()> {
    let config = load_config(config_path_override).into_diagnostic()?;
    info!("config loaded for web server");

    let (progress_tx, _) = tokio::sync::broadcast::channel(64);
    let state = AppState {
        config,
        scan_status: std::sync::Arc::new(tokio::sync::Mutex::new(
            repo_radar::adapters::web::state::ScanStatus::default(),
        )),
        last_results: std::sync::Arc::new(tokio::sync::RwLock::new(None)),
        progress_tx,
    };

    let app = web::router(state);
    let addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await.into_diagnostic()?;

    println!(
        "{} Listening on http://{addr}",
        "Serve:".green().bold()
    );

    axum::serve(listener, app).await.into_diagnostic()?;

    Ok(())
}

/// Build a `SourceAdapter` from config, supporting both `[[feeds]]` (legacy)
/// and `[[sources]]` (new multi-source).
fn build_source(
    config: &repo_radar::config::AppConfig,
    client: &reqwest::Client,
) -> SourceAdapter {
    let mut adapters: Vec<SourceAdapter> = Vec::new();

    // Legacy [[feeds]] → RSS sources (backward compat)
    if !config.feeds.is_empty() {
        adapters.push(SourceAdapter::Rss(RssSource::new(
            config.feeds.clone(),
            client.clone(),
        )));
    }

    // New [[sources]] entries
    for source_cfg in &config.sources {
        match source_cfg {
            SourceConfig::Rss { url, name } => {
                let feed = repo_radar::config::FeedConfig {
                    url: url.clone(),
                    name: name.clone(),
                };
                adapters.push(SourceAdapter::Rss(RssSource::new(
                    vec![feed],
                    client.clone(),
                )));
            }
            SourceConfig::GitHubTrending { language, since } => {
                adapters.push(SourceAdapter::GitHubTrending(GitHubTrendingSource::new(
                    language.clone(),
                    since.clone(),
                    client.clone(),
                )));
            }
            SourceConfig::HackerNews { limit } => {
                adapters.push(SourceAdapter::HackerNews(HackerNewsSource::new(
                    *limit,
                    client.clone(),
                )));
            }
            SourceConfig::Reddit { subreddits, limit } => {
                adapters.push(SourceAdapter::Reddit(RedditSource::new(
                    subreddits.clone(),
                    *limit,
                    client.clone(),
                )));
            }
        }
    }

    match adapters.len() {
        0 => SourceAdapter::Noop(NoopSource),
        1 => adapters.into_iter().next().unwrap(),
        _ => SourceAdapter::Multi(MultiSource::new(adapters)),
    }
}
