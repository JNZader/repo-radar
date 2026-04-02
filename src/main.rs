use clap::Parser;
use miette::{IntoDiagnostic, Result};
use owo_colors::OwoColorize;
use tracing::info;
use tracing_subscriber::EnvFilter;

use repo_radar::adapters::analyzer::{AnalyzerAdapter, NoopAnalyzer};
use repo_radar::adapters::crossref::{CrossRefAdapter, NoopCrossRef};
use repo_radar::adapters::filter::{FilterAdapter, GitHubMetadataFilter, NoopFilter};
use repo_radar::adapters::reporter::{NoopReporter, ReporterAdapter};
use repo_radar::adapters::source::{NoopSource, RssSource, SourceAdapter};
use repo_radar::cli::{Cli, Command, ConfigAction};
use repo_radar::config::{config_path, load_config, write_default_config};
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
        Command::Config { ref action } => handle_config(action)?,
        Command::Scan {
            dry_run,
            stage: _,
            backfill: _,
        } => handle_scan(cli.config.as_deref(), dry_run).await?,
        Command::Report {
            format: _,
            output: _,
        } => {
            eprintln!(
                "{} Report command is not yet implemented. Coming in Phase 2.",
                "TODO:".yellow().bold()
            );
        }
    }

    Ok(())
}

fn handle_config(action: &ConfigAction) -> Result<()> {
    match action {
        ConfigAction::Init => {
            let path = config_path();
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
            let config = load_config(None).into_diagnostic()?;
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

    // Source: RSS if feeds configured, otherwise Noop
    let source = if config.feeds.is_empty() {
        SourceAdapter::Noop(NoopSource)
    } else {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .into_diagnostic()?;
        SourceAdapter::Rss(RssSource::new(config.feeds.clone(), client))
    };

    // Filter: GitHub metadata if token available, otherwise Noop
    let filter = if config.general.github_token.is_some() || !config.feeds.is_empty() {
        let gh_filter = GitHubMetadataFilter::new(
            config.filter.clone(),
            config.general.github_token.as_deref(),
        )
        .into_diagnostic()?;
        FilterAdapter::GitHubMetadata(Box::new(gh_filter))
    } else {
        FilterAdapter::Noop(NoopFilter)
    };

    let analyzer = AnalyzerAdapter::Noop(NoopAnalyzer);
    let crossref = CrossRefAdapter::Noop(NoopCrossRef);
    let reporter = ReporterAdapter::Noop(NoopReporter);

    let mut pipeline = Pipeline::new(source, filter, analyzer, crossref, reporter, seen);
    let report = pipeline.run().await.into_diagnostic()?;

    println!("\n{}\n{report}", "Scan complete:".green().bold());

    Ok(())
}
