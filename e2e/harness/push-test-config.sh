#!/usr/bin/env bash
# Generate test_config.json for the Android debug build and adb-push it.
# Run after the harness stack is up and LSP_NODE_ID is known (.env).
#
# Usage: ./push-test-config.sh [LSP_NODE_ID]
#        (falls back to LSP_NODE_ID from ./.env)
set -euo pipefail
cd "$(dirname "$0")"

LSP_NODE_ID="${1:-}"
if [ -z "$LSP_NODE_ID" ] && [ -f .env ]; then
    LSP_NODE_ID=$(grep '^LSP_NODE_ID=' .env | cut -d= -f2)
fi
if [ -z "$LSP_NODE_ID" ]; then
    echo "error: LSP_NODE_ID not given and not in .env" >&2
    exit 1
fi

# 10.0.2.2 = host loopback from the Android emulator.
HOST="${HARNESS_HOST:-10.0.2.2}"
TMP=$(mktemp)
cat > "$TMP" << EOF
{
  "network": "regtest",
  "primary_chain_url": "http://${HOST}:30000",
  "fallback_chain_url": "http://${HOST}:30000",
  "lsp_pubkey": "${LSP_NODE_ID}",
  "lsp_address": "${HOST}:9735",
  "price_feed_base": "http://${HOST}:9737"
}
EOF

DEST="/sdcard/Android/data/com.stablechannels.app/files/test_config.json"
adb push "$TMP" "$DEST"
rm -f "$TMP"
echo "pushed test config (LSP ${LSP_NODE_ID:0:16}...) -> $DEST"
echo "restart the app for it to take effect:  adb shell am force-stop com.stablechannels.app"
