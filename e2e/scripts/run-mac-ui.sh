#!/usr/bin/env bash
# Open the native desktop Mac UI against the regtest harness config.
# Expected setup: backend.sh has started the harness and synced LSP_NODE_ID.
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

step "Mac desktop UI"

command -v cargo >/dev/null || die "cargo not found (Rust toolchain required for Mac UI)"

sync_lsp_node_id

LSP_NODE_ID="${LSP_NODE_ID:-}"
if [ -z "$LSP_NODE_ID" ] && [ -f "$HARNESS_DIR/.env" ]; then
    LSP_NODE_ID="$(grep -sE '^LSP_NODE_ID=' "$HARNESS_DIR/.env" | cut -d= -f2 || true)"
fi
[ -n "$LSP_NODE_ID" ] || die "LSP_NODE_ID not found; run backend.sh first"

MAC_DATA_DIR="$E2E_DIR/.mac-user-ui"
if [ "${RESET:-0}" = "1" ]; then
    case "$MAC_DATA_DIR" in
        "$E2E_DIR"/.mac-user-ui) rm -rf "$MAC_DATA_DIR" ;;
        *) die "refusing to clean unexpected Mac UI data dir: $MAC_DATA_DIR" ;;
    esac
fi
mkdir -p "$MAC_DATA_DIR"

export SC_E2E=1
export SC_HARNESS_API="$HARNESS_API"
export SC_PRICE_FEED_BASE="$HARNESS_API"
export SC_MAC_NETWORK=regtest
export SC_MAC_CHAIN_URL=http://127.0.0.1:30000
export SC_MAC_FALLBACK_CHAIN_URL=http://127.0.0.1:30000
export SC_MAC_LSP_PUBKEY="$LSP_NODE_ID"
export SC_MAC_LSP_ADDRESS=127.0.0.1:9735
export SC_MAC_USER_PORT=19738
export SC_MAC_USER_DATA_DIR="$MAC_DATA_DIR"

info "opening Stable Channels Mac UI against regtest …"
info "this command keeps running until you close the app window"
info "if the terminal pauses after price logs, check Dock/Mission Control for Stable Channels Wallet"
cargo run --features e2e --bin stable-channels
