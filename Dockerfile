# syntax=docker/dockerfile:1.7-labs
# Multi-stage build using cargo-chef for layer caching.
# Stage order: chef-base → planner → cacher → builder → final (distroless)

# ── Stage 1: shared base with cargo-chef ────────────────────────────────────
FROM rust:1.88-slim-bookworm AS chef-base
RUN apt-get update && apt-get install -y --no-install-recommends \
        pkg-config \
        libssl-dev \
        ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && cargo install cargo-chef --locked
WORKDIR /app

# ── Stage 2: generate the dependency recipe ─────────────────────────────────
FROM chef-base AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# ── Stage 3: cook (cache) dependencies ──────────────────────────────────────
FROM chef-base AS cacher
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json

# ── Stage 4: build the binary ────────────────────────────────────────────────
FROM chef-base AS builder
COPY . .
# Restore pre-built deps from cacher
COPY --from=cacher /app/target target
COPY --from=cacher /usr/local/cargo /usr/local/cargo
RUN cargo build --release --bin talon

# ── Stage 5: minimal distroless runtime ─────────────────────────────────────
FROM gcr.io/distroless/cc-debian12:nonroot AS final
LABEL org.opencontainers.image.source="https://github.com/rohirikman/talon"
LABEL org.opencontainers.image.licenses="MIT OR Apache-2.0"

COPY --from=builder /app/target/release/talon /usr/local/bin/talon

EXPOSE 7777
ENTRYPOINT ["/usr/local/bin/talon"]
