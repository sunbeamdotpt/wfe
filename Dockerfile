# Copyright Sunbeam Studios 2026
# SPDX-License-Identifier: Apache-2.0
#
# Build from the WORKSPACE ROOT, not platform/wfe. Every wfe-* crate
# lives as a workspace member in the root Cargo.toml and inherits
# [workspace.dependencies]; platform/wfe has no Cargo.toml of its own.
#
#   docker buildx build -f platform/wfe/Dockerfile -t sunbeam-wfe-server:latest .
#
# Context pruning lives in `platform/wfe/Dockerfile.dockerignore`. Keep
# that file narrow — 3p/ + apps/ + libs/ path-dep directories must stay
# included or cargo can't resolve the workspace.
#
# Using debian-slim (glibc) rather than alpine because deno_core's
# bundled v8 only ships glibc binaries — building v8 under musl from
# source is impractical and we need the full feature set (rustlang,
# buildkit, containerd, kubernetes, deno) compiled into wfe-server.

# ── Stage 1: build ──────────────────────────────────────────────
FROM rust:1-bookworm AS builder

RUN apt-get update && \
    apt-get install -y --no-install-recommends \
        protobuf-compiler libprotobuf-dev libssl-dev pkg-config \
        ca-certificates cmake && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /build
COPY . .

RUN cargo build --release --bin wfe-server \
        -p wfe-server \
        --features "wfe-yaml/rustlang,wfe-yaml/buildkit,wfe-yaml/containerd,wfe-yaml/kubernetes,wfe-yaml/deno" && \
    strip target/release/wfe-server && \
    cp target/release/wfe-server /wfe-server

# ── Stage 2: runtime ─────────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update && \
    apt-get install -y --no-install-recommends \
        ca-certificates libssl3 tini && \
    rm -rf /var/lib/apt/lists/*

RUN groupadd -r -g 1000 wfe && useradd -r -u 1000 -g wfe wfe

USER 1000:1000

COPY --from=builder /wfe-server /usr/local/bin/wfe-server

EXPOSE 50051 8080

ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/wfe-server"]
