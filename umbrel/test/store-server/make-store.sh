#!/usr/bin/env bash
# Build (or rebuild) the local community-store git repo served by
# git-smart-http.py. Copies the canonical package from umbrel/stable-channels-lsp,
# points the images at the local registry (localhost:5001) and the icon at the
# local server, and commits.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"

REPO=store-repo
rm -rf "$REPO"
mkdir -p "$REPO"
cp umbrel-app-store.yml.tmp "$REPO/umbrel-app-store.yml"
cp -R ../../stable-channels-lsp "$REPO/stable-channels-lsp"

# Local-test deltas vs the canonical package:
#  - images from the local registry instead of GHCR
#  - Lightning P2P on 19735: the e2e stack's ldk-server holds 127.0.0.1:9735
#    on the same Mac, and a bind conflict leaves the app's ldk-server without
#    ANY network attachment (crash-loops with "Network is unreachable").
TAG="${SC_STORE_IMAGE_TAG:-8598bcf}"
if [ "${SC_STORE_LOCAL_REGISTRY:-0}" = "1" ]; then
    sed -i '' 's|image: ghcr.io/toneloc/\(sc-[a-z-]*\):[a-z0-9]*|image: localhost:5001/\1:local|' \
        "$REPO/stable-channels-lsp/docker-compose.yml"
else
    sed -i '' "s|image: ghcr.io/toneloc/\(sc-[a-z-]*\):[a-z0-9]*|image: ghcr.io/toneloc/\1:${TAG}|" \
        "$REPO/stable-channels-lsp/docker-compose.yml"
fi
sed -i '' 's|"9735:9735"|"19735:9735"|' "$REPO/stable-channels-lsp/docker-compose.yml"
sed -i '' 's|^gallery: \[\]|gallery: []\nicon: http://localhost:8929/icon.png|' \
    "$REPO/stable-channels-lsp/umbrel-app.yml"

cd "$REPO"
git init -qb main
git add -A
git -c user.email=sim@local -c user.name=sim commit -qm "Stable Channels community app store (local test)"
echo "store repo built at $(pwd)"
