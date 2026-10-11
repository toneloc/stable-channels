#!/usr/bin/env bash
# Freeze the app's price-refresh loop so its quote stays at the pre-move price while
# set_price shifts the LSP's view mid-flow (deterministic quote_deviation rejection).
set -euo pipefail
cd "$(dirname "$0")/.."
PLATFORM="${1:-}"; DEVICE="${2:-}"
# Reset the harness price to the canonical baseline so the 1% move is 1%.
curl -s -X POST http://localhost:9737/price -H 'Content-Type: application/json' -d '{"price": 100000.0}' > /dev/null
if [ "$PLATFORM" = "android" ]; then
    ADB=$(command -v adb || echo "${ANDROID_HOME:-$HOME/Library/Android/sdk}/platform-tools/adb")
    SC_TEST_PRICE_REFRESH_SECS=3600 ./harness/push-test-config.sh
    "$ADB" shell am force-stop com.stablechannels.app
elif [ "$PLATFORM" = "ios" ]; then
    SC_TEST_PRICE_REFRESH_SECS=3600 IOS_SIM_UDID="$DEVICE" HARNESS_HOST=localhost \
        ./harness/push-test-config-ios.sh
    xcrun simctl terminate "$DEVICE" com.stablechannels.app 2>/dev/null || true
fi
