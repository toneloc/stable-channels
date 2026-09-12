#!/usr/bin/env bash
# Restore the standard 3s test config.
set -uo pipefail
cd "$(dirname "$0")/.."
PLATFORM="${1:-}"; DEVICE="${2:-}"
[ "$PLATFORM" = "android" ] || exit 0
ADB=$(command -v adb || echo "${ANDROID_HOME:-$HOME/Library/Android/sdk}/platform-tools/adb")
./harness/push-test-config.sh
"$ADB" shell am force-stop com.stablechannels.app
