#!/usr/bin/env bash
set -euo pipefail

VERSION="${1:?Usage: scripts/release.sh <version> [message]}"
MESSAGE="${2:-v${VERSION}}"

echo "=== Releasing v${VERSION} ==="

# Tag
git tag -a "v${VERSION}" -m "${MESSAGE}"

# Publish leaf crates
for crate in wfe-core wfe-containerd-protos wfe-buildkit-protos wfe-server-protos; do
    echo "--- Publishing ${crate} ---"
    cargo publish -p "${crate}"
done

# Middle layer
for crate in wfe-sqlite wfe-postgres wfe-opensearch wfe-valkey wfe-buildkit wfe-containerd wfe-rustlang; do
    echo "--- Publishing ${crate} ---"
    cargo publish -p "${crate}"
done

# Top layer (needs index to catch up)
sleep 10
for crate in wfe wfe-yaml; do
    echo "--- Publishing ${crate} ---"
    cargo publish -p "${crate}"
done

# Final layer
sleep 10
for crate in wfe-server wfe-deno wfe-kubernetes; do
    echo "--- Publishing ${crate} ---"
    cargo publish -p "${crate}"
done

# Build + push Docker image
echo "--- Building Docker image ---"
docker buildx build --builder sunbeam-remote --platform linux/arm64,linux/amd64 \
    -t "ghcr.io/sunbeamdotpt/wfe:${VERSION}" \
    -t "ghcr.io/sunbeamdotpt/wfe:latest" \
    --push .

echo "=== v${VERSION} released ==="
