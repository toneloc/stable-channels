#!/usr/bin/env bash
# Run the native desktop Mac smoke checks against the regtest harness config.
# Expected setup: backend.sh has started the harness and synced LSP_NODE_ID.
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

step "Mac desktop smoke"

command -v cargo >/dev/null || die "cargo not found (Rust toolchain required for Mac smoke)"

sync_lsp_node_id

LSP_NODE_ID="${LSP_NODE_ID:-}"
if [ -z "$LSP_NODE_ID" ] && [ -f "$HARNESS_DIR/.env" ]; then
    LSP_NODE_ID="$(grep -sE '^LSP_NODE_ID=' "$HARNESS_DIR/.env" | cut -d= -f2 || true)"
fi
[ -n "$LSP_NODE_ID" ] || die "LSP_NODE_ID not found; run backend.sh first"

MAC_DATA_DIR="$E2E_DIR/.mac-user"
mkdir -p "$MAC_DATA_DIR"

export SC_E2E=1
export SC_PRICE_FEED_BASE="$HARNESS_API"
export SC_MAC_NETWORK=regtest
export SC_MAC_CHAIN_URL=http://127.0.0.1:30000
export SC_MAC_FALLBACK_CHAIN_URL=http://127.0.0.1:30000
export SC_MAC_LSP_PUBKEY="$LSP_NODE_ID"
export SC_MAC_LSP_ADDRESS=127.0.0.1:9735
export SC_MAC_USER_PORT=19736
export SC_MAC_USER_DATA_DIR="$MAC_DATA_DIR"

info "cargo test --features e2e --lib desktop_config …"
cargo test --features e2e --lib desktop_config -- --nocapture

info "cargo run --features e2e --bin stable-channels -- mac-smoke …"
cargo run --features e2e --bin stable-channels -- mac-smoke

ok "Mac desktop smoke passed"
