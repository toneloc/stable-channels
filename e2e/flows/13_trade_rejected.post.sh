#!/usr/bin/env bash
# Restore the normal test config (live price refresh) and baseline price.
set -euo pipefail
cd "$(dirname "$0")/.."
PLATFORM="${1:-}"; DEVICE="${2:-}"
curl -s -X POST http://localhost:9737/price -H 'Content-Type: application/json' -d '{"price": 100000.0}' > /dev/null
if [ "$PLATFORM" = "android" ]; then
    ADB=$(command -v adb || echo "${ANDROID_HOME:-$HOME/Library/Android/sdk}/platform-tools/adb")
    ./harness/push-test-config.sh
    "$ADB" shell am force-stop com.stablechannels.app
elif [ "$PLATFORM" = "ios" ]; then
    IOS_SIM_UDID="$DEVICE" HARNESS_HOST=localhost ./harness/push-test-config-ios.sh
    xcrun simctl terminate "$DEVICE" com.stablechannels.app 2>/dev/null || true
fi
