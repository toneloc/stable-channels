#!/usr/bin/env bash
# Bring the regtest backend up and make it test-ready:
#   build+start the docker services (bitcoin-core, chain-init, block-explorer,
#   ldk-server, sc-lsp, ldk-node, optionally lsp-gui), sync the LSP node id,
#   fund + open the counterparty<->LSP channel (idempotent), and pin the mock
#   price at $100k.
# Safe to run repeatedly — a no-op once everything is already up.
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

step "Backend (docker regtest stack)"
cd "$HARNESS_DIR"
sc_require_free_space "$REPO_DIR" "${SC_E2E_MIN_FREE_GIB:-25}" "e2e backend"
sc_warn_docker_raw_size "${SC_DOCKER_RAW_WARN_GIB:-150}"
export DOCKER_BUILDKIT="${DOCKER_BUILDKIT:-1}"
sc_autotune_docker_build_limits

# Optional LSP operator GUI (web build of server/lsp-server-gui) at
# http://127.0.0.1:3003. Set E2E_LSP_GUI=1/0 to skip the prompt; non-interactive
# runs (CI) default to off.
if [ -z "${E2E_LSP_GUI:-}" ] && [ -t 0 ]; then
    read -r -p "Also start the LSP GUI container (http://127.0.0.1:3003)? [y/N] " reply
    case "$reply" in [Yy]*) E2E_LSP_GUI=1 ;; *) E2E_LSP_GUI=0 ;; esac
fi
if [ "${E2E_LSP_GUI:-0}" = "1" ]; then
    export COMPOSE_PROFILES="gui${COMPOSE_PROFILES:+,$COMPOSE_PROFILES}"
    info "LSP GUI enabled — http://127.0.0.1:3003 once up"
fi

# Reuse existing images by default (a fast, cache-safe start). Editing any repo
# file busts the `COPY . .` layer and forces a full Rust recompile, so only
# rebuild on demand: `make rebuild` (REBUILD=1) — e.g. after changing the LSP
# price hook. Missing images are still built automatically by compose.
if [ "${REBUILD:-0}" = "1" ]; then
    info "docker compose up -d --build (forced rebuild) …"
    docker compose up -d --build --remove-orphans
else
    info "docker compose up -d (reuse images; 'make rebuild' to force) …"
    docker compose up -d --remove-orphans
fi

# Stale-tip trap: after >24h idle, restarted bitcoind reports IBD=true (old
# tip timestamp), electrs then refuses to serve HTTP, and the harness panics
# on FeerateEstimationUpdateFailed. Mining one block clears the IBD flag.
if ! curl -fsS -m 3 http://127.0.0.1:30000/blocks/tip/height >/dev/null 2>&1; then
    info "esplora not serving — mining 1 block to clear bitcoind's stale-tip IBD flag …"
    docker compose exec -T bitcoin-core sh -c '
        bitcoin-cli -regtest -rpcuser=sc -rpcpassword=sc loadwallet miner 2>/dev/null
        ADDR=$(bitcoin-cli -regtest -rpcuser=sc -rpcpassword=sc -rpcwallet=miner getnewaddress)
        bitcoin-cli -regtest -rpcuser=sc -rpcpassword=sc generatetoaddress 1 "$ADDR"' >/dev/null 2>&1 || true
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
    info "recreating ldk-node (harness) to load updated LSP_NODE_ID …"
    docker compose up -d --no-deps --force-recreate ldk-node

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

if [ "${E2E_LSP_GUI:-0}" = "1" ]; then
    # The api_key is raw bytes generated by sc-lsp on first boot; the GUI wants it hex.
    gui_api_key="$(docker compose exec -T sc-lsp od -An -tx1 /data/stable-channels-lsp/regtest/api_key 2>/dev/null | tr -d ' \n' || true)"
    step "LSP GUI connection info"
    info "GUI:     http://127.0.0.1:3003"
    info "API key: ${gui_api_key:-<not ready — extract with: docker compose exec sc-lsp od -An -tx1 /data/stable-channels-lsp/regtest/api_key>}"
    info "Network: regtest"
    info "(Server URL field is ignored by the web build — nginx proxies /api/ to sc-lsp)"
fi
