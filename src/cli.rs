use std::path::PathBuf;

use clap::{Parser, Subcommand};

/// Feed-driven GitHub repo discovery engine.
#[derive(Parser, Debug)]
#[command(name = "repo-radar", version, about)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,

    /// Path to config file (overrides XDG default).
    #[arg(long, short, global = true)]
    pub config: Option<PathBuf>,

    /// Increase log verbosity (-v = debug, -vv = trace).
    #[arg(long, short, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Run the discovery pipeline.
    Scan {
        /// Preview what would happen without persisting results.
        #[arg(long)]
        dry_run: bool,

        /// Run only a specific stage (source, filter, analyze, crossref, report).
        #[arg(long)]
        stage: Option<String>,

        /// Backfill mode: process previously seen entries.
        #[arg(long)]
        backfill: bool,

        /// Also run the KB accumulator pipeline after filtering (stores all
        /// filtered candidates in the SQLite knowledge base).
        #[arg(long)]
        accumulate: bool,

        /// Path to the knowledge base SQLite file (overrides config `kb.db_path`).
        #[arg(long)]
        kb_path: Option<PathBuf>,
    },

    /// Generate reports from cached results.
    Report {
        /// Output format (markdown, json).
        #[arg(long, default_value = "markdown")]
        format: String,

        /// Output directory (overrides config).
        #[arg(long, short)]
        output: Option<PathBuf>,
    },

    /// Manage configuration.
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },

    /// Extract actionable ideas from the latest scan results.
    Ideas {
        /// Input file: path to a scan results JSON file.
        #[arg(long, short)]
        input: Option<PathBuf>,

        /// Output file for the ideas JSON.
        #[arg(long, short)]
        output: Option<PathBuf>,

        /// Minimum relevance threshold (0.0-1.0).
        #[arg(long, default_value = "0.1")]
        min_relevance: f64,

        /// Also print ideas to the console.
        #[arg(long)]
        print: bool,
    },

    /// Compare two scan snapshots and show what changed.
    Diff {
        /// ID of the earlier scan (defaults to second-latest).
        #[arg(long)]
        scan_a: Option<String>,

        /// ID of the later scan (defaults to latest).
        #[arg(long)]
        scan_b: Option<String>,
    },

    /// Compare a source repo against a target repo and generate actionable ideas.
    Compare {
        /// External repo to study for ideas (GitHub URL or owner/repo shorthand)
        #[arg(long)]
        source: String,

        /// Your own repo to improve (local path or GitHub URL)
        #[arg(long)]
        target: String,

        /// Force re-analysis even if repos are cached in the knowledge base
        #[arg(long, default_value_t = false)]
        force: bool,

        /// Save ideas to a markdown file (optional)
        #[arg(long)]
        output: Option<PathBuf>,
    },

    /// Start the web dashboard server.
    Serve {
        /// Port to listen on.
        #[arg(long, default_value = "3000")]
        port: u16,

        /// Host address to bind to.
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum ConfigAction {
    /// Create a default config file at the XDG path.
    Init,
    /// Show the currently resolved configuration.
    Show,
}
