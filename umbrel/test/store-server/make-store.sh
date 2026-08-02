#!/usr/bin/env bash
# Build (or rebuild) the local community-store git repo served by
# git-smart-http.py. Copies the canonical package from umbrel/stable-channels-lsp,
# points the images at the local registry (localhost:5001) and the icon at the
# local server, and commits.
set -euo pipefail
SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

REPO="$SCRIPT_DIR/store-repo"
[ "$REPO" = "$SCRIPT_DIR/store-repo" ] || { echo "unsafe store path" >&2; exit 1; }
rm -rf -- "$REPO"
mkdir -p "$REPO"
cp umbrel-app-store.template.yml "$REPO/umbrel-app-store.yml"
cp -R ../../stable-channels-lsp "$REPO/stable-channels-lsp"

# Local-test deltas vs the canonical package:
#  - images from the local registry instead of GHCR
sed_in_place() {
    expression="$1"
    file="$2"
    sed -E -i.bak "$expression" "$file"
    rm -f -- "$file.bak"
}

if [ "${SC_STORE_LOCAL_REGISTRY:-0}" = "1" ]; then
    sed_in_place 's|image: ghcr.io/toneloc/(sc-[a-z-]+):[^[:space:]]+|image: localhost:5001/\1:local|' \
        "$REPO/stable-channels-lsp/docker-compose.yml"
elif [ -n "${SC_STORE_IMAGE_TAG:-}" ]; then
    sed_in_place "s|image: ghcr.io/toneloc/(sc-[a-z-]+):[^[:space:]]+|image: ghcr.io/toneloc/\\1:${SC_STORE_IMAGE_TAG}|" \
        "$REPO/stable-channels-lsp/docker-compose.yml"
fi
STORE_HOST="${SC_STORE_HOST:-host.docker.internal}"
sed_in_place "s|^gallery: \[\]|gallery: []\\
icon: http://${STORE_HOST}:8929/icon.png|" \
    "$REPO/stable-channels-lsp/umbrel-app.yml"

cd "$REPO"
git init -qb main
git add -A
git -c user.email=sim@local -c user.name=sim commit -qm "Stable Channels community app store (local test)"
echo "store repo built at $(pwd)"
