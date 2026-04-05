#!/usr/bin/env bash
# Fake repoforge CLI: handles the `export` subcommand and outputs valid markdown.
# Usage: fake_repoforge.sh export -w <dir> --no-contents -q
cat << 'EOF'
# test-repo — LLM Context

## Project Overview

- **Tech stack**: Rust, tokio
- **Entry points**: src/main.rs
- **Config files**: Cargo.toml
- **Total files**: 5
- **Layers**: main

## Directory Tree

```
test-repo/
--- src/
    --- main.rs
    --- lib.rs
```

## Key Definitions

- `analyze_one` — core analysis function
- `RepoforgeAnalyzer` — main analyzer struct
- `parse_tech_stack` — extracts tech stack from markdown
EOF
