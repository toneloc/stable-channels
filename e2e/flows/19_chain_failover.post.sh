#!/usr/bin/env bash
# Restore the normal test config (working primary chain URL).
set -euo pipefail
cd "$(dirname "$0")/.."
PLATFORM="${1:-}"; DEVICE="${2:-}"
if [ "$PLATFORM" = "android" ]; then
    ADB=$(command -v adb || echo "${ANDROID_HOME:-$HOME/Library/Android/sdk}/platform-tools/adb")
    ./harness/push-test-config.sh
    "$ADB" shell am force-stop com.stablechannels.app
elif [ "$PLATFORM" = "ios" ]; then
    IOS_SIM_UDID="$DEVICE" HARNESS_HOST=localhost ./harness/push-test-config-ios.sh
    xcrun simctl terminate "$DEVICE" com.stablechannels.app 2>/dev/null || true
fi
