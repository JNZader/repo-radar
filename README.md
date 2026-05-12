# repo-radar

**Feed-driven GitHub repository discovery engine for finding ideas worth stealing on purpose.**

[![CI](https://github.com/JNZader/repo-radar/actions/workflows/ci.yml/badge.svg)](https://github.com/JNZader/repo-radar/actions/workflows/ci.yml)
[![Rust](https://img.shields.io/badge/Rust-2024_edition-000000?logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](./LICENSE)

repo-radar aggregates repositories from RSS/Atom feeds, GitHub Trending, HackerNews, Reddit, and GitHub Skills discovery, enriches them with GitHub metadata, categorizes them, and cross-references discoveries against your own repositories to surface actionable ideas.

Visuals coming soon.

## Quick Portfolio Snapshot

- Built in Rust with a CLI-first workflow and an optional Axum dashboard.
- Runs a full discovery pipeline: fetch, dedupe, filter, categorize, analyze, cross-reference, report.
- Supports multi-source ingestion, GitHub API caching, dashboard auth, and backward-compatible config loading.
- Uses a ports-and-adapters architecture to keep sources, filters, analyzers, reporters, and web adapters isolated.

## Quick Start

```sh
cargo install --path .
repo-radar config init
export REPO_RADAR_GITHUB_TOKEN="ghp_your_token_here"
export REPO_RADAR_GITHUB_USERNAME="your-github-username"
repo-radar scan
```

## Jump to Technical Docs

- [CLI commands](#cli-commands)
- [Global flags](#global-flags)
- [Configuration](#configuration)
- [Dashboard](#dashboard)
- [Source types](#source-types)
- [Pipeline](#pipeline)
- [Architecture](#architecture)

---

## Technical README

### Table of Contents

- [CLI commands](#cli-commands)
- [Global flags](#global-flags)
- [Configuration](#configuration)
- [Environment variables](#environment-variables)
- [Dashboard](#dashboard)
- [Source types](#source-types)
- [Pipeline](#pipeline)
- [Architecture](#architecture)
- [License](#license)

## CLI Commands

### `scan`

Run the full discovery pipeline: fetch, dedupe, filter, categorize, analyze, cross-reference, report.

```sh
repo-radar scan
repo-radar scan --dry-run
repo-radar scan --backfill
repo-radar scan --stage filter
repo-radar scan --accumulate
repo-radar scan --kb-path ./kb.sqlite
```

### `report`

Generate reports from cached results.

```sh
repo-radar report
repo-radar report --format json
repo-radar report --output ./my-reports
```

### `ideas`

Extract actionable ideas from scan results.

```sh
repo-radar ideas
repo-radar ideas --input results.json
repo-radar ideas --min-relevance 0.5
repo-radar ideas --print
```

### `diff`

Compare two scan snapshots and show what changed.

```sh
repo-radar diff
repo-radar diff --scan-a 2026-05-01T10:00:00Z --scan-b 2026-05-08T10:00:00Z
```

### `compare`

Compare an external repository against one of your own and generate actionable ideas.

```sh
repo-radar compare --source rust-lang/rustlings --target ~/code/my-project
repo-radar compare --source https://github.com/owner/repo --target https://github.com/me/project --output ideas.md
```

### `serve`

Start the web dashboard.

```sh
repo-radar serve
repo-radar serve --port 8080
repo-radar serve --host 0.0.0.0 --port 3000
```

### `config`

Manage configuration files.

```sh
repo-radar config init
repo-radar config show
```

## Global Flags

```sh
repo-radar --config ./custom-config.toml scan
repo-radar -v scan
repo-radar -vv scan
```

- `--config` overrides the default XDG config path.
- `-v` enables debug logging.
- `-vv` enables trace logging.

## Configuration

The config file lives at `$XDG_CONFIG_HOME/repo-radar/config.toml` by default.

```toml
[general]
data_dir = "~/.local/share/repo-radar"
log_level = "info"
backfill_batch_size = 50

[filter]
min_stars = 10
languages = ["Rust", "TypeScript"]
topics = ["cli", "htmx"]
exclude_forks = true
exclude_archived = true

[analyzer]
# repoforge_path = "/path/to/repoforge"
timeout_secs = 60
# llm_model = "openai/gpt-4o-mini"
deep_analysis_top_n = 5
deep_analysis_min_relevance = 0.15

[crossref]
own_repos = ["./exports/my-repo.json"]

[reporter]
output_dir = "./output"
format = "markdown" # markdown | json

[cache]
ttl_secs = 86400
rate_limit_threshold = 100

[kb]
enabled = false
db_path = "./kb.db"
llm_gateway_url = "http://localhost:3456"
llm_model = "openai/gpt-4o-mini"
# llm_auth_token = "sk-..."

# Legacy RSS feed syntax still works.
[[feeds]]
url = "https://example.com/feed.xml"
name = "Example Feed"

# Preferred multi-source syntax.
[[sources]]
type = "rss"
url = "https://example.com/feed.xml"
name = "Example Feed"

[[sources]]
type = "github_trending"
language = "Rust"
since = "daily" # daily | weekly | monthly

[[sources]]
type = "hackernews"
limit = 30

[[sources]]
type = "reddit"
subreddits = ["rust", "programming"]
limit = 25

[[sources]]
type = "github_skills"
limit = 30
```

### Configuration Notes

- `[[feeds]]` is the legacy RSS-only syntax and is still supported for backward compatibility.
- `[[sources]]` is the preferred syntax for mixed source types and is the clearer option for new configs.
- `REPO_RADAR_GITHUB_USERNAME` can be omitted if `gh` is installed and authenticated; repo-radar will try to resolve your username from the GitHub CLI.
- The KB pipeline can be enabled in config or triggered ad hoc with `repo-radar scan --accumulate`.

## Environment Variables

| Variable | Purpose |
|---|---|
| `REPO_RADAR_GITHUB_TOKEN` | GitHub API token for metadata enrichment, filtering, and cross-reference workflows |
| `REPO_RADAR_GITHUB_USERNAME` | GitHub username used when matching discoveries against your own repositories |
| `REPO_RADAR_DASHBOARD_TOKEN` | Optional bearer token that protects dashboard routes |
| `REPO_RADAR_LLM_API_KEY` | Optional API key for LLM-backed analysis |

## Dashboard

The `serve` command starts a dashboard built with Axum, HTMX, Askama templates, and Chart.js. It provides:

- Overview pages with category breakdowns and charts
- Browser-triggered scans with real-time SSE progress
- Repository comparison views
- Historical scan browsing
- Config inspection

By default the server binds to `127.0.0.1:3000`. Use `repo-radar serve --host 0.0.0.0` for LAN, container, or reverse-proxy setups, and set `REPO_RADAR_DASHBOARD_TOKEN` if the dashboard should not be publicly open.

## Source Types

| Source | Description | Config key |
|---|---|---|
| RSS/Atom | Any feed that links to GitHub repositories | `type = "rss"` |
| GitHub Trending | Scrapes trending repositories by language and period | `type = "github_trending"` |
| HackerNews | `Show HN` stories with GitHub links | `type = "hackernews"` |
| Reddit | Posts from configured subreddits containing GitHub links | `type = "reddit"` |
| GitHub Skills | Searches for repositories containing `SKILL.md` files | `type = "github_skills"` |

## Pipeline

The scan pipeline runs in sequence:

1. **Fetch**: pull entries from all configured sources.
2. **Dedupe**: skip previously seen repositories.
3. **Filter**: apply GitHub metadata criteria such as stars, language, topics, forks, and archived state.
4. **Categorize**: assign categories based on keywords and topics.
5. **Analyze**: extract summaries, features, and tech stack hints.
6. **Cross-reference**: match discoveries against your own repositories.
7. **Report**: output results in the configured format.

## Architecture

repo-radar follows a hexagonal architecture.

| Directory | Role |
|---|---|
| `domain/` | Core traits and models for sources, filters, analyzers, cross-reference, and reporting |
| `adapters/` | Concrete implementations for RSS, GitHub, HackerNews, Reddit, dashboard handlers, and reporters |
| `infra/` | Caching, seen-store, scan persistence, and error handling |
| `pipeline.rs` | Orchestrates the discovery pipeline |
| `config.rs` | TOML configuration loading, env overlays, and validation |

Pipeline stages are trait-based and include Noop-friendly boundaries that make isolated testing and incremental implementation practical.

## License

MIT
