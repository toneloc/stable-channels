#!/usr/bin/env bash
# Point the app's PRIMARY chain source at a dead port so startup must fail over
# to the fallback (the harness explorer at :30000). Restored by the .post.sh.
set -euo pipefail
cd "$(dirname "$0")/.."
PLATFORM="${1:-}"; DEVICE="${2:-}"
if [ "$PLATFORM" = "android" ]; then
    ADB=$(command -v adb || echo "${ANDROID_HOME:-$HOME/Library/Android/sdk}/platform-tools/adb")
    # Port 1 on the host loopback: refused immediately, no route surprises.
    SC_TEST_PRIMARY_CHAIN_URL="http://10.0.2.2:1" ./harness/push-test-config.sh
    "$ADB" shell am force-stop com.stablechannels.app
elif [ "$PLATFORM" = "ios" ]; then
    SC_TEST_PRIMARY_CHAIN_URL="http://localhost:1" IOS_SIM_UDID="$DEVICE" HARNESS_HOST=localhost \
        ./harness/push-test-config-ios.sh
    xcrun simctl terminate "$DEVICE" com.stablechannels.app 2>/dev/null || true
fi
