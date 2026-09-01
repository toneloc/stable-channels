#!/usr/bin/env bash
# Restore the normal test config (live price refresh) and baseline price.
set -euo pipefail
cd "$(dirname "$0")/.."
ADB=$(command -v adb || echo "${ANDROID_HOME:-$HOME/Library/Android/sdk}/platform-tools/adb")
[ "${1:-}" = "android" ] || exit 0
./harness/push-test-config.sh
curl -s -X POST http://localhost:9737/price -H 'Content-Type: application/json' -d '{"price": 100000.0}' > /dev/null
"$ADB" shell am force-stop com.stablechannels.app
