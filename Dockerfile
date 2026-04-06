# Stage 1: Build
FROM rust:alpine AS builder

RUN apk add --no-cache musl-dev protobuf-dev openssl-dev openssl-libs-static pkgconfig

WORKDIR /build
COPY . .

# Configure the sunbeam cargo registry (workspace deps reference it)
RUN mkdir -p .cargo && printf '[registries.sunbeam]\nindex = "sparse+https://src.sunbeam.pt/api/packages/studio/cargo/"\n' > .cargo/config.toml

RUN cargo build --release --bin wfe-server \
    -p wfe-server \
    --features "wfe-yaml/rustlang,wfe-yaml/buildkit,wfe-yaml/containerd,wfe-yaml/kubernetes" \
    && strip target/release/wfe-server

# Stage 2: Runtime
FROM alpine:3.21

RUN apk add --no-cache ca-certificates tini

COPY --from=builder /build/target/release/wfe-server /usr/local/bin/wfe-server

RUN adduser -D -u 1000 wfe
USER wfe

EXPOSE 50051 8080

ENTRYPOINT ["tini", "--"]
CMD ["wfe-server"]
