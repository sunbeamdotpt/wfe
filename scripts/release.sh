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
    cargo publish -p "${crate}" --registry sunbeam
done

# Middle layer
for crate in wfe-sqlite wfe-postgres wfe-opensearch wfe-valkey wfe-buildkit wfe-containerd wfe-rustlang; do
    echo "--- Publishing ${crate} ---"
    cargo publish -p "${crate}" --registry sunbeam
done

# Top layer (needs index to catch up)
sleep 10
for crate in wfe wfe-yaml; do
    echo "--- Publishing ${crate} ---"
    cargo publish -p "${crate}" --registry sunbeam
done

# Final layer
sleep 10
for crate in wfe-server wfe-deno wfe-kubernetes; do
    echo "--- Publishing ${crate} ---"
    cargo publish -p "${crate}" --registry sunbeam
done

# Push git
git push origin mainline
git push origin "v${VERSION}"

# Create Gitea release from changelog
release_body=$(awk -v ver="${VERSION}" '/^## \[/{if(found)exit;if(index($0,"["ver"]"))found=1;next}found{print}' CHANGELOG.md)
tea release create --tag "v${VERSION}" --title "v${VERSION}" --note "${release_body}" || echo "(release already exists)"

# Build + push Docker image
echo "--- Building Docker image ---"
docker buildx build --builder sunbeam-remote --platform linux/amd64 \
    -t "src.sunbeam.pt/studio/wfe:${VERSION}" \
    -t "src.sunbeam.pt/studio/wfe:latest" \
    --push .

echo "=== v${VERSION} released ==="
