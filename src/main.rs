use clap::Parser;
use miette::{IntoDiagnostic, Result};
use owo_colors::OwoColorize;
use tracing::info;
use tracing_subscriber::EnvFilter;

use repo_radar::adapters::analyzer::{AnalyzerAdapter, NoopAnalyzer, RepoforgeAnalyzer};
use repo_radar::adapters::categorizer::{CategorizerAdapter, KeywordCategorizer};
use repo_radar::adapters::compare::LlmCompareService;
use repo_radar::adapters::crossref::{CrossRefAdapter, NoopCrossRef};
use repo_radar::adapters::crossref::github_crossref::GitHubCrossRef;
use repo_radar::adapters::filter::{FilterAdapter, GitHubMetadataFilter, NoopFilter};
use repo_radar::adapters::github::readme_fetcher::GithubReadmeFetcher;
use repo_radar::adapters::idea_extractor::KeywordIdeaExtractor;
use repo_radar::adapters::kb::LlmKbAnalyzer;
use repo_radar::adapters::reporter::{
    ConsoleReporter, JsonReporter, MarkdownReporter, ReporterAdapter,
};
use repo_radar::adapters::source::{
    GitHubSkillsSource, GitHubTrendingSource, HackerNewsSource, MultiSource, NoopSource,
    RedditSource, RssSource,
    SourceAdapter,
};
use repo_radar::adapters::web::{self, AppState};
use repo_radar::cli::{Cli, Command, ConfigAction};
use repo_radar::config::{config_path, load_config, write_default_config, SourceConfig};
use repo_radar::domain::compare::CompareService;
use repo_radar::domain::idea_extractor::IdeaExtractor;
use repo_radar::domain::kb::KbAnalyzer;
use repo_radar::domain::model::CrossRefResult;
use repo_radar::infra::cache::RepoCache;
use repo_radar::infra::repoforge::RepoforgeRunner;
use repo_radar::infra::seen::SeenStore;
use repo_radar::infra::sqlite_kb::SqliteKb;
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
            accumulate,
            ref kb_path,
        } => handle_scan(cli.config.as_deref(), dry_run, accumulate, kb_path.clone()).await?,
        Command::Ideas {
            ref input,
            ref output,
            min_relevance,
            print,
        } => {
            handle_ideas(cli.config.as_deref(), input.as_deref(), output.as_deref(), min_relevance, print).await?
        }
        Command::Serve { port, ref host } => {
            handle_serve(cli.config.as_deref(), port, host).await?
        }
        Command::Report {
            ref format,
            ref output,
        } => {
            handle_report(cli.config.as_deref(), format, output.as_deref()).await?;
        }
        Command::Diff {
            ref scan_a,
            ref scan_b,
        } => {
            handle_diff(
                cli.config.as_deref(),
                scan_a.as_deref(),
                scan_b.as_deref(),
            )
            .await?;
        }
        Command::Compare {
            ref source,
            ref target,
            force,
            ref output,
        } => {
            handle_compare(
                cli.config.as_deref(),
                source,
                target,
                force,
                output.as_deref(),
            )
            .await?;
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

async fn handle_scan(
    config_path_override: Option<&std::path::Path>,
    dry_run: bool,
    accumulate: bool,
    kb_path_override: Option<std::path::PathBuf>,
) -> Result<()> {
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

    // Fetch own repos for semantic scoring (best-effort — empty vec if unavailable)
    let own_repos = match &crossref {
        CrossRefAdapter::GitHub(gh) => gh.fetch_own_repos_summary().await.unwrap_or_default(),
        CrossRefAdapter::Noop(_) => vec![],
    };

    let mut pipeline = Pipeline::new(source, filter, categorizer, analyzer, crossref, reporter, seen, None)
        .with_analyzer_config(config.analyzer.clone())
        .with_own_repos(own_repos);

    let (report, results) = if accumulate {
        use repo_radar::kb_pipeline::build_kb_pipeline;

        let kb_pipeline = build_kb_pipeline(
            &config.kb,
            kb_path_override,
            config.analyzer.repoforge_path.clone(),
            config.analyzer.timeout_secs,
        )
        .into_diagnostic()?;

        let (pipeline_report, crossref_results, kb_report) = pipeline
            .run_with_kb(&kb_pipeline)
            .await
            .into_diagnostic()?;

        println!(
            "\n{}\n{kb_report}",
            "KB accumulation complete:".green().bold()
        );

        (pipeline_report, crossref_results)
    } else {
        pipeline.run().await.into_diagnostic()?
    };

    // Persist scan results for later use by `report` and `ideas` commands
    let store = repo_radar::infra::scan_store::ScanResultStore::new(
        config.general.data_dir.join("results"),
    );
    if let Err(e) = store.save(&results) {
        eprintln!(
            "{} Failed to cache scan results: {e}",
            "Warning:".yellow().bold()
        );
    } else {
        info!(count = results.len(), "scan results cached");
    }

    println!("\n{}\n{report}", "Scan complete:".green().bold());

    Ok(())
}

async fn handle_ideas(
    config_path_override: Option<&std::path::Path>,
    input: Option<&std::path::Path>,
    output: Option<&std::path::Path>,
    min_relevance: f64,
    print: bool,
) -> Result<()> {
    let config = load_config(config_path_override).into_diagnostic()?;
    info!("config loaded for ideas extraction");

    let crossref_results: Vec<CrossRefResult> = if let Some(input_path) = input {
        info!(path = %input_path.display(), "loading scan results from file");
        let content = tokio::fs::read_to_string(input_path)
            .await
            .into_diagnostic()?;
        serde_json::from_str(&content).into_diagnostic()?
    } else {
        let output_dir = &config.reporter.output_dir;
        let mut latest: Option<(std::time::SystemTime, std::path::PathBuf)> = None;

        if output_dir.exists() {
            let entries = std::fs::read_dir(output_dir).into_diagnostic()?;
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "json")
                    && path.file_name().is_some_and(|n| n.to_string_lossy().starts_with("report-"))
                    && let Ok(meta) = path.metadata()
                    && let Ok(modified) = meta.modified()
                    && latest.as_ref().is_none_or(|(t, _)| modified > *t)
                {
                    latest = Some((modified, path));
                }
            }
        }

        if let Some((_, path)) = latest {
            info!(path = %path.display(), "loading most recent scan results");
            let content = tokio::fs::read_to_string(&path)
                .await
                .into_diagnostic()?;
            serde_json::from_str(&content).into_diagnostic()?
        } else {
            eprintln!(
                "{} No scan results found. Run {} first, or provide --input <file>.",
                "Error:".red().bold(),
                "repo-radar scan".cyan().bold()
            );
            return Ok(());
        }
    };

    info!(count = crossref_results.len(), "crossref results loaded");

    let extractor = KeywordIdeaExtractor::new(min_relevance);
    let idea_report = extractor.extract(&crossref_results).into_diagnostic()?;

    let output_path = if let Some(path) = output {
        path.to_path_buf()
    } else {
        let timestamp = chrono::Utc::now().timestamp();
        config.reporter.output_dir.join(format!("ideas-{timestamp}.json"))
    };

    if let Some(parent) = output_path.parent() {
        tokio::fs::create_dir_all(parent).await.into_diagnostic()?;
    }

    let json = serde_json::to_string_pretty(&idea_report).into_diagnostic()?;
    tokio::fs::write(&output_path, &json).await.into_diagnostic()?;

    println!(
        "{} {} ideas extracted from {} repos, targeting {} of your repos",
        "Ideas:".green().bold(),
        idea_report.total_ideas.to_string().bold(),
        idea_report.repos_analyzed.to_string().bold(),
        idea_report.target_repos_involved.to_string().bold(),
    );
    println!(
        "{} Written to {}",
        "Output:".cyan().bold(),
        output_path.display()
    );

    if print {
        println!("\n{:=<80}", "".bold());
        for (i, idea) in idea_report.ideas.iter().enumerate() {
            println!(
                "\n{} {} [{}] (relevance: {:.0}%, impact: {})",
                format!("#{}", i + 1).bold().cyan(),
                idea.kind,
                idea.category,
                idea.relevance * 100.0,
                idea.impact,
            );
            println!(
                "  {} {} -> {}",
                "Source:".dimmed(),
                idea.source_repo.white(),
                idea.target_repo.yellow(),
            );
            println!("  {}", idea.description);
            if !idea.source_features.is_empty() {
                println!(
                    "  {} {}",
                    "Features:".dimmed(),
                    idea.source_features.join(", ")
                );
            }
            if !idea.relevant_tech.is_empty() {
                println!(
                    "  {} {}",
                    "Tech:".dimmed(),
                    idea.relevant_tech.join(", ")
                );
            }
        }
        println!("\n{:=<80}", "".bold());
    }

    Ok(())
}

async fn handle_report(
    config_path_override: Option<&std::path::Path>,
    format: &str,
    output: Option<&std::path::Path>,
) -> Result<()> {
    use repo_radar::domain::reporter::Reporter;

    let config = load_config(config_path_override).into_diagnostic()?;
    let store = repo_radar::infra::scan_store::ScanResultStore::new(
        config.general.data_dir.join("results"),
    );

    let scans = store.list().into_diagnostic()?;
    if scans.is_empty() {
        eprintln!(
            "{} No cached scan results found. Run {} first.",
            "Error:".red().bold(),
            "repo-radar scan".cyan().bold(),
        );
        return Ok(());
    }

    // Load the most recent scan
    let latest = &scans[0];
    let results = store.load(&latest.id).into_diagnostic()?;
    info!(id = %latest.id, count = results.len(), "loaded cached scan results");

    let output_dir = output
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| config.reporter.output_dir.clone());

    let reporter = match format {
        "json" => ReporterAdapter::Json(JsonReporter::new(output_dir.clone())),
        "console" => ReporterAdapter::Console(ConsoleReporter::new()),
        _ => ReporterAdapter::Markdown(MarkdownReporter::new(output_dir.clone())),
    };

    reporter.report(&results).await.into_diagnostic()?;

    println!(
        "{} Report generated ({} format, {} results) in {}",
        "Done:".green().bold(),
        format.bold(),
        results.len().to_string().bold(),
        output_dir.display(),
    );

    Ok(())
}

async fn handle_serve(
    config_path_override: Option<&std::path::Path>,
    port: u16,
    host: &str,
) -> Result<()> {
    let config = load_config(config_path_override).into_diagnostic()?;
    info!("config loaded for web server");

    let scan_store = std::sync::Arc::new(repo_radar::infra::scan_store::ScanResultStore::new(
        config.general.data_dir.join("results"),
    ));

    // Pre-load the most recent scan results so the dashboard shows data on startup
    let cached_results = match scan_store.load_latest() {
        Ok(Some(results)) => {
            info!(count = results.len(), "loaded cached scan results for dashboard");
            Some(results)
        }
        Ok(None) => {
            info!("no cached scan results found");
            None
        }
        Err(e) => {
            tracing::warn!(%e, "failed to load cached scan results");
            None
        }
    };

    let (progress_tx, _) = tokio::sync::broadcast::channel(64);
    let state = AppState {
        config,
        scan_status: std::sync::Arc::new(tokio::sync::Mutex::new(
            repo_radar::adapters::web::state::ScanStatus::default(),
        )),
        last_results: std::sync::Arc::new(tokio::sync::RwLock::new(cached_results)),
        progress_tx,
        scan_store,
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

async fn handle_diff(
    config_path_override: Option<&std::path::Path>,
    scan_a_id: Option<&str>,
    scan_b_id: Option<&str>,
) -> Result<()> {
    use repo_radar::domain::diff::compute_diff;

    let config = load_config(config_path_override).into_diagnostic()?;
    let store = repo_radar::infra::scan_store::ScanResultStore::new(
        config.general.data_dir.join("results"),
    );

    let scans = store.list().into_diagnostic()?;

    if scans.len() < 2 {
        eprintln!(
            "{} Need at least 2 saved scans to diff. Run {} first.",
            "Error:".red().bold(),
            "repo-radar scan".cyan().bold(),
        );
        return Ok(());
    }

    // Resolve IDs — default: second-latest vs latest (scans are newest-first)
    let meta_b = if let Some(id) = scan_b_id {
        scans
            .iter()
            .find(|s| s.id == id)
            .cloned()
            .ok_or_else(|| miette::miette!("scan not found: {id}"))?
    } else {
        scans[0].clone()
    };

    let meta_a = if let Some(id) = scan_a_id {
        scans
            .iter()
            .find(|s| s.id == id)
            .cloned()
            .ok_or_else(|| miette::miette!("scan not found: {id}"))?
    } else {
        scans[1].clone()
    };

    let results_a = store.load(&meta_a.id).into_diagnostic()?;
    let results_b = store.load(&meta_b.id).into_diagnostic()?;

    let diff = compute_diff(meta_a.clone(), meta_b.clone(), &results_a, &results_b);

    // ── Header ──────────────────────────────────────────────────────────────
    println!(
        "\n{} {} → {}",
        "Scan diff:".bold().cyan(),
        meta_a.id.dimmed(),
        meta_b.id.dimmed(),
    );
    println!(
        "  {} {} new  {} removed  {} changed  {} unchanged\n",
        "Summary:".bold(),
        diff.new_repos.len().to_string().green().bold(),
        diff.removed_repos.len().to_string().red().bold(),
        diff.changed_repos.len().to_string().yellow().bold(),
        diff.unchanged_count.to_string().dimmed(),
    );

    // ── New repos ────────────────────────────────────────────────────────────
    if !diff.new_repos.is_empty() {
        println!("{}", "New Repositories:".green().bold());
        for r in &diff.new_repos {
            let c = &r.analysis.candidate;
            println!(
                "  {} {}/{} (relevance: {:.0}%)",
                "+".green().bold(),
                c.owner.white(),
                c.repo_name.white(),
                r.overall_relevance * 100.0,
            );
        }
        println!();
    }

    // ── Removed repos ────────────────────────────────────────────────────────
    if !diff.removed_repos.is_empty() {
        println!("{}", "Removed Repositories:".red().bold());
        for r in &diff.removed_repos {
            let c = &r.analysis.candidate;
            println!(
                "  {} {}/{} (was: {:.0}%)",
                "-".red().bold(),
                c.owner.dimmed(),
                c.repo_name.dimmed(),
                r.overall_relevance * 100.0,
            );
        }
        println!();
    }

    // ── Changed repos ────────────────────────────────────────────────────────
    if !diff.changed_repos.is_empty() {
        println!("{}", "Changed Repositories:".yellow().bold());
        for repo_diff in &diff.changed_repos {
            let c = &repo_diff.result.analysis.candidate;
            let delta_str = if repo_diff.score_delta >= 0.0 {
                format!("+{:.1}%", repo_diff.score_delta * 100.0)
                    .green()
                    .bold()
                    .to_string()
            } else {
                format!("{:.1}%", repo_diff.score_delta * 100.0)
                    .red()
                    .bold()
                    .to_string()
            };
            println!(
                "  ~ {}/{} score: {delta_str}",
                c.owner.white(),
                c.repo_name.white(),
            );
            if !repo_diff.new_ideas.is_empty() {
                for idea in &repo_diff.new_ideas {
                    println!("      {} {}", "idea:".cyan(), idea);
                }
            }
        }
        println!();
    }

    Ok(())
}

async fn handle_compare(
    config_path_override: Option<&std::path::Path>,
    source: &str,
    target: &str,
    force: bool,
    output: Option<&std::path::Path>,
) -> Result<()> {
    let config = load_config(config_path_override).into_diagnostic()?;
    info!("config loaded for compare command");

    // -- Initialize shared components ------------------------------------------
    let kb_db_path = config.kb.db_path.clone();
    let kb = SqliteKb::open(&kb_db_path).into_diagnostic()?;

    let llm_analyzer = LlmKbAnalyzer::new(
        config.kb.llm_gateway_url.clone(),
        config.kb.llm_model.clone(),
        config.kb.llm_auth_token.clone(),
    );
    let compare_service = LlmCompareService::new(
        config.kb.llm_gateway_url.clone(),
        config.kb.llm_model.clone(),
        config.kb.llm_auth_token.clone(),
    );
    let fetcher = GithubReadmeFetcher::new(config.general.github_token.clone());

    // -- A) Resolve SOURCE repo (always GitHub) --------------------------------
    let source_analysis = {
        let ctx = fetcher
            .fetch(source)
            .await
            .into_diagnostic()
            .map_err(|e| miette::miette!("fetching source repo {source}: {e}"))?;

        let needs = if force {
            true
        } else {
            kb.needs_analysis(&ctx.owner, &ctx.repo_name, ctx.pushed_at)
                .into_diagnostic()?
        };

        if needs {
            info!(repo = %format!("{}/{}", ctx.owner, ctx.repo_name), "analyzing source repo");
            let mut analysis = llm_analyzer
                .analyze(&ctx.context, &ctx.owner, &ctx.repo_name)
                .await
                .into_diagnostic()?;
            analysis.is_own = false;
            analysis.owner = ctx.owner.clone();
            analysis.repo_name = ctx.repo_name.clone();
            analysis.pushed_at = ctx.pushed_at;
            kb.upsert(&analysis).into_diagnostic()?;
            analysis
        } else {
            let id = format!("{}/{}", ctx.owner, ctx.repo_name);
            kb.get(&id)
                .into_diagnostic()?
                .ok_or_else(|| miette::miette!("source repo {id} not found in KB after cache check"))?
        }
    };

    // -- B) Resolve TARGET repo (local path or GitHub URL) --------------------
    let target_analysis = {
        // Detect GitHub URL or owner/repo shorthand vs local path
        let is_github = target.starts_with("https://")
            || target.starts_with("http://")
            || (target.contains('/') && !target.contains(std::path::MAIN_SEPARATOR) && !std::path::Path::new(target).exists());

        if is_github {
            let ctx = fetcher
                .fetch(target)
                .await
                .into_diagnostic()
                .map_err(|e| miette::miette!("fetching target repo {target}: {e}"))?;

            let needs = if force {
                true
            } else {
                kb.needs_analysis(&ctx.owner, &ctx.repo_name, ctx.pushed_at)
                    .into_diagnostic()?
            };

            if needs {
                info!(repo = %format!("{}/{}", ctx.owner, ctx.repo_name), "analyzing target repo (github)");
                let mut analysis = llm_analyzer
                    .analyze(&ctx.context, &ctx.owner, &ctx.repo_name)
                    .await
                    .into_diagnostic()?;
                analysis.is_own = true;
                analysis.owner = ctx.owner.clone();
                analysis.repo_name = ctx.repo_name.clone();
                analysis.pushed_at = ctx.pushed_at;
                kb.upsert(&analysis).into_diagnostic()?;
                analysis
            } else {
                let id = format!("{}/{}", ctx.owner, ctx.repo_name);
                kb.get(&id)
                    .into_diagnostic()?
                    .ok_or_else(|| miette::miette!("target repo {id} not found in KB after cache check"))?
            }
        } else {
            // Local path
            let local_path = std::path::PathBuf::from(target);
            if !local_path.exists() {
                return Err(miette::miette!("target path does not exist: {target}"));
            }

            // Derive owner/repo_name from path
            let repo_name = local_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_owned();
            let owner = "local".to_owned();
            let id = format!("local/{repo_name}");

            // Get pushed_at from git log
            let pushed_at = tokio::process::Command::new("git")
                .args(["-C", target, "log", "-1", "--format=%cI"])
                .output()
                .await
                .ok()
                .and_then(|out| {
                    if out.status.success() {
                        String::from_utf8(out.stdout).ok()
                    } else {
                        None
                    }
                })
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s.trim()).ok())
                .map(|dt| dt.with_timezone(&chrono::Utc));

            let needs = if force {
                true
            } else {
                kb.needs_analysis(&owner, &repo_name, pushed_at)
                    .into_diagnostic()?
            };

            if needs {
                // Run repoforge export to get context
                let repoforge_path = config
                    .analyzer
                    .repoforge_path
                    .clone()
                    .unwrap_or_else(|| std::path::PathBuf::from("repoforge"));
                let timeout_secs = config.analyzer.timeout_secs;
                let runner = RepoforgeRunner::new(
                    repoforge_path,
                    std::time::Duration::from_secs(timeout_secs),
                );
                info!(path = %local_path.display(), "running repoforge export on target");
                let context = runner
                    .export(&local_path)
                    .await
                    .into_diagnostic()
                    .map_err(|e| miette::miette!("repoforge export failed for {target}: {e}"))?;

                let mut analysis = llm_analyzer
                    .analyze(&context, &owner, &repo_name)
                    .await
                    .into_diagnostic()?;
                analysis.is_own = true;
                analysis.owner = owner.clone();
                analysis.repo_name = repo_name.clone();
                analysis.pushed_at = pushed_at;
                analysis.url = format!("file://{}", local_path.display());
                kb.upsert(&analysis).into_diagnostic()?;
                analysis
            } else {
                kb.get(&id)
                    .into_diagnostic()?
                    .ok_or_else(|| miette::miette!("target repo {id} not found in KB after cache check"))?
            }
        }
    };

    // -- D) Compare and format output ------------------------------------------
    let result = compare_service
        .compare(&source_analysis, &target_analysis)
        .await
        .into_diagnostic()?;

    let mut markdown = format!(
        "# Ideas: {} → {}\n\n",
        result.source_id, result.target_id
    );

    for idea in &result.ideas {
        markdown.push_str(&format!(
            "## {}\n**Effort**: {} | **Impact**: {}\n\n{}\n\n---\n\n",
            idea.title, idea.effort, idea.impact, idea.description
        ));
    }

    println!("{markdown}");

    if let Some(out_path) = output {
        if let Some(parent) = out_path.parent() {
            if !parent.as_os_str().is_empty() {
                tokio::fs::create_dir_all(parent).await.into_diagnostic()?;
            }
        }
        tokio::fs::write(out_path, &markdown)
            .await
            .into_diagnostic()?;
        println!(
            "{} Written to {}",
            "Output:".cyan().bold(),
            out_path.display()
        );
    }

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
                    limit: None,
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
            SourceConfig::GitHubSkills { limit } => {
                adapters.push(SourceAdapter::GitHubSkills(GitHubSkillsSource::new(
                    client.clone(),
                    config.general.github_token.clone(),
                    *limit,
                )));
            }
        }
    }

    match adapters.len() {
        0 => SourceAdapter::Noop(NoopSource),
        1 => adapters.into_iter().next().expect("len checked == 1"),
        _ => SourceAdapter::Multi(MultiSource::new(adapters)),
    }
}
