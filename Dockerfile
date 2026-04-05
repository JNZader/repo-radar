# ── Stage 1: Rust build ───────────────────────────────────
FROM rust:1.86-slim AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
# Cache dependencies layer — build empty src first
RUN mkdir src && echo 'fn main(){}' > src/main.rs && \
    cargo build --release 2>/dev/null || true && \
    rm -rf src

COPY src/ src/
COPY tests/ tests/
# Touch main.rs so cargo rebuilds the binary (not just deps)
RUN touch src/main.rs && cargo build --release

# ── Stage 2: Runtime ──────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    git \
    python3 \
    python3-pip \
    curl && \
    rm -rf /var/lib/apt/lists/*

# Install uv (Python package manager) and repoforge
RUN curl -LsSf https://astral.sh/uv/install.sh | sh
ENV PATH="/root/.local/bin:$PATH"
RUN uv tool install repoforge

# Copy repo-radar binary
COPY --from=builder /app/target/release/repo-radar /usr/local/bin/repo-radar

# Data directory for seen.json, reports, kb.sqlite, config
RUN mkdir -p /data/reports
VOLUME ["/data"]

ENV REPO_RADAR_DATA_DIR=/data

EXPOSE 3000

CMD ["repo-radar", "serve", "--host", "0.0.0.0"]
