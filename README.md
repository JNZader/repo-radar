# repo-radar

Read this in: [English](README.md) · [Español](README.es.md)

**Feed-driven GitHub repository discovery engine for finding ideas worth stealing on purpose.**

[![CI](https://github.com/JNZader/repo-radar/actions/workflows/ci.yml/badge.svg)](https://github.com/JNZader/repo-radar/actions/workflows/ci.yml)
[![Rust](https://img.shields.io/badge/Rust-2024_edition-000000?logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](./LICENSE)

repo-radar aggregates repositories from RSS/Atom feeds, GitHub Trending, HackerNews, Reddit, and GitHub Skills discovery, enriches them with GitHub metadata, categorizes them, and cross-references discoveries against your own repositories to surface actionable ideas. It ships as a single CLI binary (`repo-radar`) with an optional Axum web dashboard.

- **Version:** 0.1.0 · **Edition:** Rust 2024 · **License:** MIT
- **Async runtime:** Tokio (multi-thread)
- **HTTP:** `reqwest` (rustls) · **GitHub API:** `octocrab` · **Feeds:** `feed-rs`

## Overview

- Built in Rust with a CLI-first workflow and an optional web dashboard.
- Runs a discovery pipeline: fetch → dedupe → filter → categorize → analyze → cross-reference → report.
- Ingests multiple source types, caches GitHub API responses on disk, and supports optional dashboard bearer-token auth.
- Accepts both a legacy `[[feeds]]` config block and the newer `[[sources]]` multi-source block.
- Uses a ports-and-adapters (hexagonal) architecture to keep sources, filters, analyzers, reporters, and web adapters isolated.

## Quick Start

```sh
cargo install --path .
repo-radar config init
export REPO_RADAR_GITHUB_TOKEN="ghp_your_token_here"
export REPO_RADAR_GITHUB_USERNAME="your-github-username"
repo-radar scan
```

`repo-radar config init` writes a commented `config.toml` to the XDG config path. Running `scan` before a config exists prints a first-run message instead of failing.

## Table of Contents

- [CLI commands](#cli-commands)
- [Global flags](#global-flags)
- [Configuration](#configuration)
- [Environment variables](#environment-variables)
- [Dashboard](#dashboard)
- [Source types](#source-types)
- [Pipeline](#pipeline)
- [Architecture](#architecture)
- [Development](#development)
- [License](#license)

## CLI Commands

Available subcommands: `scan`, `report`, `ideas`, `diff`, `compare`, `serve`, `config`.

### `scan`

Run the discovery pipeline and cache the results for later use by `report`, `ideas`, and `diff`.

```sh
repo-radar scan
repo-radar scan --dry-run          # print the resolved config and exit without fetching
repo-radar scan --accumulate       # also run the KB pipeline, storing candidates in SQLite
repo-radar scan --kb-path ./kb.sqlite
```

Flags:

- `--dry-run` — print the fully resolved configuration and exit; no network calls.
- `--accumulate` — after filtering, run the knowledge-base pipeline and persist candidates to the SQLite KB.
- `--kb-path <PATH>` — override the KB SQLite path (config key `kb.db_path`).
- `--stage <STAGE>` / `--backfill` — accepted by the parser but **not yet implemented**; they are currently no-ops.

### `report`

Regenerate a report from the most recent cached scan (no re-fetching).

```sh
repo-radar report
repo-radar report --format json     # markdown (default) | json | console
repo-radar report --output ./my-reports
```

### `ideas`

Extract actionable ideas from cached scan results (or an explicit JSON file).

```sh
repo-radar ideas
repo-radar ideas --input results.json
repo-radar ideas --min-relevance 0.5   # default: 0.1
repo-radar ideas --print               # also print ideas to the console
```

Without `--input`, the most recent `report-*.json` in the reporter output directory is used.

### `diff`

Compare two cached scan snapshots and show new, removed, and changed repositories.

```sh
repo-radar diff                                  # second-latest vs latest
repo-radar diff --scan-a <scan-id> --scan-b <scan-id>
```

Scan IDs come from the cached scans in the data directory. With no IDs, `diff` compares the second-latest scan against the latest. Requires at least two saved scans.

### `compare`

Compare an external repository against one of your own and generate ideas. This path uses an LLM gateway (see the `[kb]` config) and a knowledge-base cache.

```sh
repo-radar compare --source rust-lang/rustlings --target ~/code/my-project
repo-radar compare --source https://github.com/owner/repo --target https://github.com/me/project --output ideas.md
```

- `--source` — external repo to study (GitHub URL or `owner/repo` shorthand).
- `--target` — your repo to improve (local path or GitHub URL). Local targets are analyzed via `repoforge` export.
- `--force` — re-analyze even if the repos are already cached in the KB.
- `--output <FILE>` — also write the generated ideas to a markdown file.

### `serve`

Start the web dashboard.

```sh
repo-radar serve
repo-radar serve --port 8080                    # default: 3000
repo-radar serve --host 0.0.0.0 --port 3000     # default host: 127.0.0.1
```

### `config`

```sh
repo-radar config init     # create a default config at the XDG path (no-op if it exists)
repo-radar config show     # print the fully resolved configuration as TOML
```

## Global Flags

These apply to every subcommand:

```sh
repo-radar --config ./custom-config.toml scan
repo-radar -v scan
repo-radar -vv scan
```

- `--config, -c <PATH>` — use a specific config file instead of the XDG default.
- `-v` — debug logging. `-vv` — trace logging. (Overridable via the `RUST_LOG` env filter.)

## Configuration

The config file lives at `$XDG_CONFIG_HOME/repo-radar/config.toml` by default (created by `config init`). The block below documents every supported key; the generated template is a smaller commented starting point.

```toml
[general]
data_dir = "~/.local/share/repo-radar"   # defaults to the XDG data dir
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
format = "markdown"   # markdown | json | console

[cache]
ttl_secs = 86400
rate_limit_threshold = 100
# cache_dir = "./cache"   # defaults to data_dir/cache

[kb]
enabled = false
db_path = "kb.db"
llm_gateway_url = "http://localhost:3456"
llm_model = "openai/gpt-4o-mini"
# llm_auth_token = "sk-..."

# Legacy RSS-only syntax (still supported for backward compatibility).
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
since = "daily"   # daily | weekly | monthly

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

### Configuration notes

- `[[feeds]]` is the legacy RSS-only syntax and is still supported. `[[sources]]` is the preferred syntax for mixed source types. Both can coexist in one file.
- Config values are validated on load: invalid feed/source URLs, unknown reporter formats, out-of-range timeouts and batch sizes, and unknown log levels are all reported (all problems at once). Suspicious or placeholder GitHub tokens produce a warning.
- `REPO_RADAR_GITHUB_USERNAME` and `REPO_RADAR_GITHUB_TOKEN` can be omitted if the GitHub CLI (`gh`) is installed and authenticated — repo-radar falls back to `gh auth token` and `gh api user` to resolve them.
- The KB pipeline runs automatically when `[kb] enabled = true`, or ad hoc via `repo-radar scan --accumulate`.

## Environment Variables

| Variable | Purpose |
|---|---|
| `REPO_RADAR_GITHUB_TOKEN` | GitHub API token for metadata enrichment, filtering, and cross-reference. Falls back to `gh auth token` if unset. |
| `REPO_RADAR_GITHUB_USERNAME` | GitHub username used to match discoveries against your own repositories. Falls back to `gh api user` if unset. |
| `REPO_RADAR_DASHBOARD_TOKEN` | Optional bearer token that, when set, protects all dashboard routes. |
| `REPO_RADAR_LLM_API_KEY` | Optional LLM API key. Loaded into config and shown (masked) on the dashboard config page. Note: the `compare`/KB LLM calls authenticate with `[kb] llm_auth_token`, not this variable. |
| `RUST_LOG` | Standard `tracing` env filter; overrides the `-v`/`-vv` verbosity level. |

## Dashboard

The `serve` command starts a dashboard built with Axum, HTMX, and Askama templates; charts are rendered with Chart.js (loaded from a CDN, so the charts need network access). It provides:

- An overview page with category and language breakdowns and charts.
- Browser-triggered scans with real-time progress over Server-Sent Events (`/api/scan/events`).
- Repository comparison views (`/compare/{owner}/{repo}`).
- Historical scan browsing and per-report detail (`/reports`, `/reports/{id}`).
- Scan-to-scan diff views (`/diff`, `/diff/{id_a}/{id_b}`).
- A config inspection page with secrets masked.

By default the server binds to `127.0.0.1:3000`. Use `repo-radar serve --host 0.0.0.0` for LAN, container, or reverse-proxy setups, and set `REPO_RADAR_DASHBOARD_TOKEN` to require a bearer token on every route.

## Source Types

| Source | Description | Config key |
|---|---|---|
| RSS/Atom | Any feed that links to GitHub repositories | `type = "rss"` |
| GitHub Trending | Trending repositories by language and period (`daily`/`weekly`/`monthly`) | `type = "github_trending"` |
| HackerNews | Stories with GitHub links | `type = "hackernews"` |
| Reddit | Posts from configured subreddits containing GitHub links | `type = "reddit"` |
| GitHub Skills | Repositories containing `SKILL.md` files | `type = "github_skills"` |

Multiple sources are fanned out concurrently and merged; a single configured source is used directly, and no sources falls back to a no-op.

## Pipeline

The `scan` pipeline runs in sequence:

1. **Fetch** — pull entries from all configured sources.
2. **Dedupe** — skip previously seen repositories (tracked in `seen.json`).
3. **Filter** — apply GitHub metadata criteria (stars, language, topics, forks, archived state). Requires a GitHub token; otherwise this stage is a no-op.
4. **Categorize** — assign categories from keywords and topics.
5. **Analyze** — extract summaries, features, and tech-stack hints. Uses `repoforge` when `analyzer.repoforge_path` is set; otherwise a no-op analyzer.
6. **Cross-reference** — match discoveries against your own repositories. Requires a GitHub username; otherwise a no-op.
7. **Report** — write results in the configured format.

Results are cached to the data directory so `report`, `ideas`, and `diff` can run without re-fetching.

## Architecture

repo-radar follows a ports-and-adapters (hexagonal) architecture.

| Path | Role |
|---|---|
| `src/domain/` | Core traits and models for sources, filters, categorizers, analyzers, cross-reference, scoring, and reporting |
| `src/adapters/` | Concrete implementations: RSS, GitHub Trending, HackerNews, Reddit, GitHub Skills, reporters, KB, and the web dashboard |
| `src/infra/` | Caching, seen-store, scan persistence, SQLite KB, rate limiting, and error handling |
| `src/pipeline.rs` | Orchestrates the discovery pipeline |
| `src/kb_pipeline.rs` | Builds the optional knowledge-base accumulation pipeline |
| `src/config.rs` | TOML config loading, env-var overlays, and validation |

Each pipeline stage is trait-based, with no-op adapters at every boundary so stages can be swapped out or tested in isolation.

## Development

```sh
cargo build            # debug build
cargo test             # run the unit + integration test suite
cargo clippy           # lints
cargo run -- scan      # run from source
```

A `Dockerfile` and `docker-compose.dev.yml` are included for containerized runs, and CI is defined in `.github/workflows/ci.yml`.

## License

MIT. (The `LICENSE` file is referenced by the badge above; add one to the repository root if it is missing.)
