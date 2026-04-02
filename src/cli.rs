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
