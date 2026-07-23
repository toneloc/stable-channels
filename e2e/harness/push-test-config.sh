#!/usr/bin/env bash
# Generate test_config.json for the Android debug build and adb-push it.
# Run after the harness stack is up and LSP_NODE_ID is known (.env).
#
# Usage: ./push-test-config.sh [LSP_NODE_ID]
#        (falls back to LSP_NODE_ID from ./.env)
set -euo pipefail
cd "$(dirname "$0")"

# Resolve adb even when it's not on PATH.
if ! command -v adb > /dev/null 2>&1; then
    ADB="${ANDROID_HOME:-$HOME/Library/Android/sdk}/platform-tools/adb"
else
    ADB=$(command -v adb)
fi

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
trap 'rm -f "$TMP"' EXIT
cat > "$TMP" << EOF
{
  "network": "regtest",
  "primary_chain_url": "http://${HOST}:30000",
  "fallback_chain_url": "http://${HOST}:30000",
  "lsp_pubkey": "${LSP_NODE_ID}",
  "lsp_address": "${HOST}:9735",
  "price_feed_base": "http://${HOST}:9737",
  "push_register_url": "http://${HOST}:3002/api/register-push",
  "channel_exists_url": "http://${HOST}:3002/api/channel-exists",
  "disable_send_auth": true,
  "sync_interval_secs": 10
}
EOF

DEST_DIR="/sdcard/Android/data/com.stablechannels.app/files"
DEST="$DEST_DIR/test_config.json"
"$ADB" shell mkdir -p "$DEST_DIR" >/dev/null
"$ADB" push "$TMP" "$DEST"
echo "pushed test config (LSP ${LSP_NODE_ID:0:16}...) -> $DEST"
echo "restart the app for it to take effect:  adb shell am force-stop com.stablechannels.app"
