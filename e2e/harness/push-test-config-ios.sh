#!/usr/bin/env bash
# Write test_config.json into the iOS simulator app's Documents directory.
# Run after: app installed on a booted simulator, harness up, .env has LSP_NODE_ID.
set -euo pipefail
cd "$(dirname "$0")"

LSP_NODE_ID="${1:-}"
if [ -z "$LSP_NODE_ID" ] && [ -f .env ]; then
    LSP_NODE_ID=$(grep '^LSP_NODE_ID=' .env | cut -d= -f2)
fi
[ -z "$LSP_NODE_ID" ] && { echo "error: LSP_NODE_ID not given and not in .env" >&2; exit 1; }

BUNDLE_ID="com.stablechannels.app"
# iOS simulator reaches the host directly via localhost.
HOST="${HARNESS_HOST:-localhost}"

CONTAINER=$(xcrun simctl get_app_container booted "$BUNDLE_ID" data)
mkdir -p "$CONTAINER/Documents"
cat > "$CONTAINER/Documents/test_config.json" << EOF
{
  "network": "regtest",
  "primary_chain_url": "http://${HOST}:30000",
  "fallback_chain_url": "http://${HOST}:30000",
  "lsp_pubkey": "${LSP_NODE_ID}",
  "lsp_address": "${HOST}:9735",
  "price_feed_base": "http://${HOST}:9737",
  "disable_send_auth": true
}
EOF
echo "wrote test config (LSP ${LSP_NODE_ID:0:16}...) -> $CONTAINER/Documents/"
echo "restart the app to apply:  xcrun simctl terminate booted $BUNDLE_ID"
