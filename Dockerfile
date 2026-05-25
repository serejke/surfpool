# syntax=docker/dockerfile:1.7
#
# Multi-stage Rust build optimized for fast incremental rebuilds.
#
# How the caching works:
#   * Toolchain pinned via `RUST_VERSION` (matches /rust-toolchain).
#   * `cargo-chef` splits dependency compilation from source compilation:
#       - `planner` derives a recipe.json from Cargo.{toml,lock}.
#       - `builder` cooks just the deps in one layer; that layer is reused
#         as long as Cargo.{toml,lock} don't change, even if every source
#         file changes.
#   * BuildKit `--mount=type=cache` keeps the cargo registry, git db, and
#     target/ around on the host's buildx cache across `docker build`
#     invocations, so a one-line source edit recompiles only the touched
#     crate instead of the full workspace. Cache mounts are not baked into
#     the image, so the final image stays small.
#
# Requires BuildKit (default in modern Docker). Build with:
#   docker buildx build -t surfpool:local .

ARG RUST_VERSION=1.89
ARG CARGO_CHEF_VERSION=0.1.77

FROM rust:${RUST_VERSION}-bullseye AS chef
ARG CARGO_CHEF_VERSION
ENV CARGO_NET_GIT_FETCH_WITH_CLI=true
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
        pkg-config \
        libssl-dev \
        libclang-11-dev \
        wget \
        tar \
    && rm -rf /var/lib/apt/lists/*
# Install the toolchain components our rust-toolchain file requires so the
# first real cargo invocation doesn't pay for it.
RUN rustup component add llvm-tools rustc-dev
RUN --mount=type=cache,id=surfpool-cargo-registry,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,id=surfpool-cargo-git,target=/usr/local/cargo/git,sharing=locked \
    cargo install cargo-chef --locked --version ${CARGO_CHEF_VERSION}
WORKDIR /src/surfpool

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /src/surfpool/recipe.json recipe.json
# Cook only the dependency graph. Reused as long as Cargo.{toml,lock} are
# unchanged, regardless of source edits.
RUN --mount=type=cache,id=surfpool-cargo-registry,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,id=surfpool-cargo-git,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,id=surfpool-target,target=/src/surfpool/target,sharing=locked \
    cargo chef cook --release --recipe-path recipe.json --bin surfpool

COPY . .
# Build the surfpool binary and copy it out of the target/ cache mount in
# the same RUN, since cache mounts don't persist into the layer.
RUN --mount=type=cache,id=surfpool-cargo-registry,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,id=surfpool-cargo-git,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,id=surfpool-target,target=/src/surfpool/target,sharing=locked \
    cargo build --release --bin surfpool --locked \
    && cp target/release/surfpool /usr/local/bin/surfpool

FROM debian:bullseye-slim AS runtime

# Bind on all container interfaces, but advertise localhost by default so
# local Docker users get client-friendly URLs without extra configuration.
ENV SURFPOOL_NETWORK_HOST=0.0.0.0 \
    SURFPOOL_PUBLIC_HOST=127.0.0.1

RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
        libssl1.1 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /usr/local/bin/surfpool /usr/local/bin/surfpool
COPY docker/entrypoint.sh /usr/local/bin/entrypoint.sh
RUN chmod +x /usr/local/bin/entrypoint.sh

WORKDIR /workspace

EXPOSE 8899 8900 18488

HEALTHCHECK --interval=10s --timeout=3s --start-period=30s --retries=3 \
    CMD curl --fail --silent --output /dev/null \
        --header 'Content-Type: application/json' \
        --data '{"jsonrpc":"2.0","id":1,"method":"getHealth"}' \
        http://127.0.0.1:8899

ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
