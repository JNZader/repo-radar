<div align="center">

# 📡 repo-radar

**Feed-driven GitHub repo discovery engine with cross-reference analysis**

[![Rust](https://img.shields.io/badge/Rust-2024_edition-000000?logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](./LICENSE)
[![Build](https://img.shields.io/badge/build-passing-brightgreen)]()

*A pipeline that aggregates repos from RSS, GitHub Trending, HackerNews & Reddit — enriches them with GitHub metadata — then cross-references discoveries against your own repositories to surface actionable ideas.*

<!-- TODO: Add hero screenshot here -->
<!-- <img src="docs/assets/hero.png" alt="repo-radar terminal output" width="700" /> -->

</div>

---

## ✨ Features

- 🔍 **Multi-source ingestion** — RSS/Atom feeds, GitHub Trending, HackerNews, Reddit
- ⭐ **GitHub metadata filtering** — stars, language, topics, fork/archived exclusion
- 🏷️ **Auto-categorization** — AI Agents, Security, DevOps, RAG/Search, Testing, UI/UX & more
- 🔗 **Cross-reference analysis** — match discoveries against your own repos
- 💡 **Idea extraction** — relevance scoring and impact assessment
- 🔄 **Deduplication** — persistent seen-store across runs
- 📊 **Multi-format reports** — Markdown, JSON, or console
- 🖥️ **Web dashboard** — HTMX + Askama templates + Chart.js
- 🔒 **Auth support** — optional bearer token for dashboard access
- ⚡ **Caching** — GitHub API response caching with configurable TTL

<!-- TODO: Add screenshot of terminal output showing repo discovery -->
<!-- <img src="docs/assets/terminal-scan.png" alt="repo-radar scan output" width="600" /> -->

<!-- TODO: Add screenshot of dashboard/analysis view -->
<!-- <img src="docs/assets/dashboard.png" alt="repo-radar dashboard" width="600" /> -->

## 🛠️ Tech Stack

| Layer | Technology | Purpose |
|-------|-----------|---------|
| Language | **Rust** (2024 edition) | Performance, safety, async |
| Async Runtime | Tokio | Multi-threaded async I/O |
| CLI | Clap | Command-line interface |
| GitHub API | Octocrab | GitHub REST API client |
| HTTP | reqwest | HTTP client (rustls) |
| Feed Parsing | feed-rs | RSS & Atom feed parser |
| Error Handling | miette | Fancy diagnostic errors |
| Progress | indicatif | Terminal progress bars |
| Web | Axum + HTMX + Askama | Dashboard server & templates |
| Database | rusqlite | SQLite for seen-store & knowledge base |
| Testing | proptest + insta | Property testing & snapshot testing |

## 🚀 Quick Start

### Install

```sh
cargo install --path .
```

### Configure

```sh
repo-radar config init        # Creates config at $XDG_CONFIG_HOME/repo-radar/config.toml
export REPO_RADAR_GITHUB_TOKEN="ghp_your_token_here"
export REPO_RADAR_GITHUB_USERNAME="your-github-username"
```

### Run a scan

```sh
repo-radar scan
```

## 📋 CLI Commands

### `scan` — Full discovery pipeline

```sh
repo-radar scan                    # Fetch → dedupe → filter → categorize → analyze → report
repo-radar scan --dry-run          # Preview resolved config without running
repo-radar scan --backfill         # Re-process previously seen entries
repo-radar scan --stage filter     # Run only a specific stage
```

### `ideas` — Extract actionable ideas

```sh
repo-radar ideas                         # Use latest scan results
repo-radar ideas --input results.json    # Use specific file
repo-radar ideas --min-relevance 0.5     # Filter by relevance threshold
repo-radar ideas --print                 # Print ideas to console
```

### `report` — Generate reports

```sh
repo-radar report                    # Markdown (default)
repo-radar report --format json
repo-radar report --format console
repo-radar report --output ./my-reports
```

### `serve` — Web dashboard

```sh
repo-radar serve                    # Default: 127.0.0.1:3000
repo-radar serve --port 8080
```

### `config` — Manage configuration

```sh
repo-radar config init              # Create default config
repo-radar config show              # Print resolved config
```

## ⚙️ Configuration

Config is TOML at `$XDG_CONFIG_HOME/repo-radar/config.toml`:

```toml
[general]
data_dir = "~/.local/share/repo-radar"
log_level = "info"

[filter]
min_stars = 10
languages = ["Rust", "TypeScript"]
exclude_forks = true
exclude_archived = true

[analyzer]
timeout_secs = 60

[reporter]
output_dir = "./output"
format = "markdown"

[cache]
ttl_secs = 86400

[[sources]]
type = "rss"
url = "https://example.com/feed.xml"
name = "Example Feed"

[[sources]]
type = "github_trending"
language = "Rust"
since = "daily"

[[sources]]
type = "hackernews"
limit = 30

[[sources]]
type = "reddit"
subreddits = ["rust", "programming"]
limit = 25
```

### Environment Variables

| Variable | Purpose |
|---|---|
| `REPO_RADAR_GITHUB_TOKEN` | GitHub API token |
| `REPO_RADAR_GITHUB_USERNAME` | Your GitHub username for cross-reference |
| `REPO_RADAR_DASHBOARD_TOKEN` | Bearer token for dashboard auth (optional) |
| `REPO_RADAR_LLM_API_KEY` | API key for LLM-based analysis (optional) |

## 🔄 Pipeline

```
Fetch → Dedupe → Filter → Categorize → Analyze → Cross-reference → Report
```

Each stage is trait-based with Noop implementations for testing and incremental development.

## 🏛️ Architecture

Hexagonal (ports & adapters) design:

| Directory | Role |
|-----------|------|
| `domain/` | Core traits (`Source`, `Filter`, `Categorizer`, `Analyzer`, `CrossRef`, `Reporter`) and models |
| `adapters/` | Concrete implementations (RSS, GitHub, HackerNews, Reddit, dashboard, reporters) |
| `infra/` | Caching, seen-store, scan persistence, error types |
| `pipeline.rs` | Orchestrates the discovery pipeline |
| `config.rs` | TOML config loading and validation |

## 📡 Source Types

| Source | Description | Config key |
|--------|-------------|------------|
| RSS/Atom | Any feed linking to GitHub repos | `type = "rss"` |
| GitHub Trending | Trending repos by language & period | `type = "github_trending"` |
| HackerNews | "Show HN" stories with GitHub links | `type = "hackernews"` |
| Reddit | Subreddit posts containing GitHub links | `type = "reddit"` |

## 📄 License

MIT