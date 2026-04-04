# repo-radar

Feed-driven GitHub repository discovery engine with cross-reference analysis.

repo-radar aggregates repositories from multiple sources (RSS feeds, GitHub Trending, HackerNews, Reddit), enriches them with GitHub metadata, categorizes them, and cross-references discoveries against your own repositories to surface actionable ideas.

## Features

- Multi-source ingestion: RSS/Atom feeds, GitHub Trending, HackerNews, Reddit
- GitHub metadata filtering (stars, language, topics, fork/archived exclusion)
- Automatic categorization (AI Agents, Security, DevOps, RAG/Search, Testing, UI/UX, and more)
- Cross-reference analysis against your own GitHub repositories
- Idea extraction with relevance scoring and impact assessment
- Deduplication via persistent seen-store
- Reports in Markdown, JSON, or console output
- Web dashboard with HTMX, Askama templates, and Chart.js
- Optional bearer token authentication for the dashboard
- GitHub API response caching with configurable TTL

## Quick Start

### Install

```sh
cargo install --path .
```

### Initialize configuration

```sh
repo-radar config init
```

This creates a default config file at `$XDG_CONFIG_HOME/repo-radar/config.toml`.

### Set environment variables

```sh
export REPO_RADAR_GITHUB_TOKEN="ghp_your_token_here"
export REPO_RADAR_GITHUB_USERNAME="your-github-username"
```

### Run a scan

```sh
repo-radar scan
```

## CLI Commands

### scan

Run the full discovery pipeline: fetch, dedupe, filter, categorize, analyze, cross-reference, report.

```sh
repo-radar scan
repo-radar scan --dry-run       # Preview resolved config without running
repo-radar scan --backfill      # Re-process previously seen entries
repo-radar scan --stage filter  # Run only a specific stage
```

### report

Generate reports from cached scan results.

```sh
repo-radar report                    # Markdown (default)
repo-radar report --format json
repo-radar report --format console
repo-radar report --output ./my-reports
```

### ideas

Extract actionable ideas from scan results. Compares discovered repos against your own to suggest feature adoptions, gap fills, tech adoptions, and pattern transfers.

```sh
repo-radar ideas                         # Use latest scan results
repo-radar ideas --input results.json    # Use specific file
repo-radar ideas --min-relevance 0.5     # Filter by relevance threshold
repo-radar ideas --print                 # Print ideas to console
```

### serve

Start the web dashboard.

```sh
repo-radar serve                    # Default: 127.0.0.1:3000
repo-radar serve --port 8080
repo-radar serve --host 0.0.0.0
```

### config

Manage configuration files.

```sh
repo-radar config init    # Create default config
repo-radar config show    # Print resolved config
```

### Global flags

```sh
repo-radar --config ./custom-config.toml scan   # Override config path
repo-radar -v scan                                # Debug logging
repo-radar -vv scan                               # Trace logging
```

## Configuration

The config file is TOML and lives at `$XDG_CONFIG_HOME/repo-radar/config.toml` by default.

```toml
[general]
data_dir = "~/.local/share/repo-radar"
log_level = "info"
backfill_batch_size = 50

[filter]
min_stars = 10
languages = ["Rust", "TypeScript"]
topics = []
exclude_forks = true
exclude_archived = true

[analyzer]
# repoforge_path = "/path/to/repoforge"
timeout_secs = 60

[reporter]
output_dir = "./output"
format = "markdown"    # markdown | json | console

[cache]
ttl_secs = 86400       # 24 hours

# Legacy feed syntax
[[feeds]]
url = "https://example.com/feed.xml"
name = "Example Feed"

# Multi-source syntax (preferred)
[[sources]]
type = "rss"
url = "https://example.com/feed.xml"
name = "Example Feed"

[[sources]]
type = "github_trending"
language = "Rust"
since = "daily"         # daily | weekly | monthly

[[sources]]
type = "hackernews"
limit = 30

[[sources]]
type = "reddit"
subreddits = ["rust", "programming"]
limit = 25
```

### Environment variables

| Variable | Purpose |
|---|---|
| `REPO_RADAR_GITHUB_TOKEN` | GitHub API token for metadata filtering and cross-reference |
| `REPO_RADAR_GITHUB_USERNAME` | Your GitHub username for cross-referencing your repos |
| `REPO_RADAR_DASHBOARD_TOKEN` | Bearer token to protect the web dashboard |
| `REPO_RADAR_LLM_API_KEY` | API key for LLM-based analysis (optional) |

## Source Types

| Source | Description | Config key |
|---|---|---|
| RSS/Atom | Any feed that links to GitHub repositories | `type = "rss"` |
| GitHub Trending | Scrapes trending repos by language and time period | `type = "github_trending"` |
| HackerNews | "Show HN" stories with GitHub links | `type = "hackernews"` |
| Reddit | Posts from specified subreddits containing GitHub links | `type = "reddit"` |

## Pipeline

The scan pipeline runs in sequence:

1. **Fetch** -- Pull entries from all configured sources
2. **Dedupe** -- Skip previously seen repositories
3. **Filter** -- Apply GitHub metadata criteria (stars, language, forks, archived)
4. **Categorize** -- Assign categories based on keywords and topics
5. **Analyze** -- Extract summaries, features, and tech stack
6. **Cross-reference** -- Match discoveries against your own repositories
7. **Report** -- Output results in the configured format

## Dashboard

The `serve` command starts a web dashboard built with Axum, HTMX, Askama templates, and Tailwind CSS. It provides:

- Overview of scan results with category breakdown and charts
- Trigger scans from the browser with real-time SSE progress
- Compare individual repos against your own
- Browse historical scan reports
- Config viewer

Set `REPO_RADAR_DASHBOARD_TOKEN` to require bearer token authentication. Without it, the dashboard is open but only binds to localhost.

## Architecture

repo-radar follows hexagonal (ports and adapters) architecture:

- `domain/` -- Core traits (`Source`, `Filter`, `Categorizer`, `Analyzer`, `CrossRef`, `Reporter`) and models
- `adapters/` -- Concrete implementations for each port (RSS, GitHub, HackerNews, Reddit, web dashboard, reporters)
- `infra/` -- Infrastructure concerns (caching, seen-store, scan persistence, error types)
- `pipeline.rs` -- Orchestrates the full discovery pipeline
- `config.rs` -- TOML configuration loading and validation

All pipeline stages are trait-based with Noop implementations for testing and incremental development.

## License

MIT
