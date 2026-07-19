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
sed -i '' \
    -e 's|image: ghcr.io/toneloc/\(sc-[a-z-]*\):[a-z0-9]*|image: localhost:5001/\1:local|' \
    "$REPO/stable-channels-lsp/docker-compose.yml"
sed -i '' 's|^gallery: \[\]|gallery: []\nicon: http://localhost:8929/icon.png|' \
    "$REPO/stable-channels-lsp/umbrel-app.yml"

cd "$REPO"
git init -qb main
git add -A
git -c user.email=sim@local -c user.name=sim commit -qm "Stable Channels community app store (local test)"
echo "store repo built at $(pwd)"
