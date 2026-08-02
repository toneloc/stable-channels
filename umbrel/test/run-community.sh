#!/usr/bin/env bash
# Exercise the real Umbrel community-store install path without publishing the
# Stable Channels images or store repository to an external service.
set -euo pipefail

TEST_DIR="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(CDPATH= cd -- "$TEST_DIR/../.." && pwd)"
STORE_SERVER_DIR="$TEST_DIR/store-server"
LOCAL_REGISTRY="${SC_UMBREL_LOCAL_REGISTRY:-localhost:5001}"
UMBREL_DEV_CONTAINER="${SC_UMBREL_DEV_CONTAINER:-umbrel-dev}"
STORE_HOST="${SC_STORE_HOST:-172.17.0.1}"
STORE_BIND="${SC_STORE_BIND:-0.0.0.0}"

die() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

require_command() {
    command -v "$1" >/dev/null 2>&1 || die "missing required command: $1"
}

require_docker() {
    require_command docker
    docker info >/dev/null 2>&1 || die "Docker is not running or is not accessible"
}

container_running() {
    [ "$(docker container inspect -f '{{.State.Running}}' "$1" 2>/dev/null || true)" = "true" ]
}

require_umbrel_dev() {
    require_docker
    container_running "$UMBREL_DEV_CONTAINER" ||
        die "umbrel-dev is not running; start it with npm run dev in the Umbrel repository"
    docker exec "$UMBREL_DEV_CONTAINER" docker container inspect sc-local-registry >/dev/null 2>&1 ||
        die "Umbrel Dev's local registry is not available"
}

build_images() {
    require_docker
    docker build -f "$REPO_ROOT/umbrel/docker/Dockerfile.ldk-server" \
        -t sc-ldk-server-umbrel-test "$REPO_ROOT"
    docker build -f "$REPO_ROOT/umbrel/docker/Dockerfile.sc-lsp" \
        -t sc-lsp-umbrel-test "$REPO_ROOT"
    docker build -f "$REPO_ROOT/umbrel/docker/Dockerfile.lsp-gui" \
        -t sc-lsp-gui-umbrel-test "$REPO_ROOT"
}

images_exist() {
    docker image inspect sc-ldk-server-umbrel-test >/dev/null 2>&1 &&
        docker image inspect sc-lsp-umbrel-test >/dev/null 2>&1 &&
        docker image inspect sc-lsp-gui-umbrel-test >/dev/null 2>&1
}

publish_images() {
    require_umbrel_dev
    while read -r source target; do
        docker save "$source" | docker exec -i "$UMBREL_DEV_CONTAINER" docker load >/dev/null
        docker exec "$UMBREL_DEV_CONTAINER" docker tag "$source" "$LOCAL_REGISTRY/$target:local"
        docker exec "$UMBREL_DEV_CONTAINER" docker push "$LOCAL_REGISTRY/$target:local"
    done <<'IMAGES'
sc-ldk-server-umbrel-test sc-ldk-server
sc-lsp-umbrel-test sc-lsp
sc-lsp-gui-umbrel-test sc-lsp-gui
IMAGES
}

build_store() {
    SC_STORE_LOCAL_REGISTRY=1 \
    SC_STORE_HOST="$STORE_HOST" \
        "$STORE_SERVER_DIR/make-store.sh"
    printf 'community store URL: %s\n' \
        "http://${STORE_HOST}:8929/stable-channels-app-store/.git"
}

serve_store() {
    require_command python3
    [ -d "$STORE_SERVER_DIR/store-repo/.git" ] ||
        die "store repo is missing; run ./run-community.sh prepare first"
    exec python3 "$STORE_SERVER_DIR/git-smart-http.py" \
        "$STORE_SERVER_DIR/store-repo" 8929 "$STORE_BIND"
}

case "${1:-help}" in
    prepare)
        require_umbrel_dev
        images_exist || build_images
        publish_images
        build_store
        ;;
    rebuild)
        build_images
        publish_images
        build_store
        ;;
    serve-store)
        serve_store
        ;;
    *)
        printf '%s\n' \
            'usage: ./run-community.sh {prepare|rebuild|serve-store}' \
            '' \
            'prepare      build missing images, publish them to Umbrel Dev, and generate the store' \
            'rebuild      rebuild all images before preparing the store' \
            'serve-store  serve the local Git store in the foreground (keep this terminal open)'
        ;;
esac
