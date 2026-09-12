#!/usr/bin/env bash
# Slow the splice confirmation monitor to 120s so the confirmation poller
# deterministically wins the completion race (the mainnet ordering).
set -euo pipefail
cd "$(dirname "$0")/.."
PLATFORM="${1:-}"; DEVICE="${2:-}"
[ "$PLATFORM" = "android" ] || exit 0
ADB=$(command -v adb || echo "${ANDROID_HOME:-$HOME/Library/Android/sdk}/platform-tools/adb")
SC_TEST_SYNC_INTERVAL_SECS=120 ./harness/push-test-config.sh
"$ADB" shell am force-stop com.stablechannels.app
