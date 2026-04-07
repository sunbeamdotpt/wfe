# Stage 1: Build
#
# Using debian-slim (glibc) rather than alpine because deno_core's bundled v8
# only ships glibc binaries — building v8 under musl from source is impractical
# and we need the full feature set (rustlang, buildkit, containerd, kubernetes,
# deno) compiled into wfe-server.
FROM rust:1-bookworm AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
        protobuf-compiler libprotobuf-dev libssl-dev pkg-config ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build
COPY . .

# Configure the sunbeam cargo registry (workspace deps reference it)
RUN mkdir -p .cargo && printf '[registries.sunbeam]\nindex = "sparse+https://src.sunbeam.pt/api/packages/studio/cargo/"\n' > .cargo/config.toml

RUN cargo build --release --bin wfe-server \
    -p wfe-server \
    --features "wfe-yaml/rustlang,wfe-yaml/buildkit,wfe-yaml/containerd,wfe-yaml/kubernetes,wfe-yaml/deno" \
    && strip target/release/wfe-server

# Stage 2: Runtime
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates tini libssl3 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/wfe-server /usr/local/bin/wfe-server

RUN useradd -u 1000 -m wfe
USER wfe

EXPOSE 50051 8080

ENTRYPOINT ["tini", "--"]
CMD ["wfe-server"]
