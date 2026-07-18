#!/usr/bin/env bash
# Bring the regtest backend up and make it test-ready:
#   build+start the 5 docker services, sync the LSP node id, fund + open the
#   counterparty<->LSP channel (idempotent), and pin the mock price at $100k.
# Safe to run repeatedly — a no-op once everything is already up.
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

step "Backend (docker regtest stack)"
cd "$HARNESS_DIR"

# Reuse existing images by default (a fast, cache-safe start). Editing any repo
# file busts the `COPY . .` layer and forces a full Rust recompile, so only
# rebuild on demand: `make rebuild` (REBUILD=1) — e.g. after changing the LSP
# price hook. Missing images are still built automatically by compose.
if [ "${REBUILD:-0}" = "1" ]; then
    info "docker compose up -d --build (forced rebuild) …"
    docker compose up -d --build
else
    info "docker compose up -d (reuse images; 'make rebuild' to force) …"
    docker compose up -d
fi

info "waiting for harness API …"
for _ in $(seq 1 60); do
    curl -fsS "$HARNESS_API/info" >/dev/null 2>&1 && break || sleep 2
done
curl -fsS "$HARNESS_API/info" >/dev/null 2>&1 || die "harness API never came up (see: make logs)"
ok "harness API up"

# A fresh ldk-server volume mints a new node id — keep .env in step with it.
info "syncing LSP node id …"
live_lsp_node_id="$(wait_for_lsp_node_id)" \
    || die "ldk-server node id never appeared in logs (see: make logs)"
sync_lsp_node_id "$live_lsp_node_id"

if [ "${SYNC_LSP_NODE_ID_UPDATED:-0}" = "1" ]; then
    info "recreating harness to load updated LSP_NODE_ID …"
    docker compose up -d --no-deps --force-recreate harness

    info "waiting for harness API after LSP node id sync …"
    for _ in $(seq 1 60); do
        curl -fsS "$HARNESS_API/info" >/dev/null 2>&1 && break || sleep 2
    done
    curl -fsS "$HARNESS_API/info" >/dev/null 2>&1 \
        || die "harness API never came back after LSP node id sync (see: make logs)"
    ok "harness API reloaded LSP node id"
fi

info "bootstrapping counterparty↔LSP channel (idempotent) …"
curl -fsS -X POST "$HARNESS_API/bootstrap" \
    -H 'Content-Type: application/json' -d '{}' >/dev/null \
    || die "bootstrap failed (LSP onchain funds? see: make logs)"

info "waiting for channel to be ready …"
for _ in $(seq 1 60); do
    harness_channel_ready && break || sleep 2
done
harness_channel_ready || die "no ready channel after bootstrap (see: make logs)"
ok "channel ready"

# Both app and LSP price off the harness feeds; pin canonical $100k.
harness_set_price 100000
ok "mock price pinned at \$100,000"

# Wait for the LSP itself to price off the mock feed (not real ~\$64k) — flow
# 03's settlement needs app and LSP to agree. Recreated containers take a few
# seconds to log their first median.
info "waiting for LSP to price from mock feed …"
lsp_price=""
for _ in $(seq 1 15); do
    lsp_price="$(docker compose logs sc-lsp 2>/dev/null | grep 'Median BTC/USD' | tail -1 || true)"
    case "$lsp_price" in *100000*) break ;; esac
    sleep 2
done
case "$lsp_price" in
    *100000*) ok "LSP prices from mock feed" ;;
    *64*|*63*|*65*) die "LSP still prices from REAL feed (${lsp_price##*:}) — sc-lsp image lacks the E2E price-feed feature; run 'make rebuild' and retry" ;;
    *) info "LSP median not logged yet (${lsp_price:-none}); flows allow catch-up time" ;;
esac
