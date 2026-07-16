#!/usr/bin/env bash
# E2E suite orchestrator: prepares fresh app state WITHOUT losing the regtest
# overrides (Maestro's clearState would wipe test_config.json along with app
# data), then runs the flows.
#
# Usage:
#   ./run.sh                          # fresh state, full suite
#   ./run.sh flows/02_btc_to_usd.yaml # fresh state, one flow
#   RESET=0 ./run.sh flows/04_*.yaml  # keep current wallet state
set -euo pipefail
cd "$(dirname "$0")"

if ! command -v adb > /dev/null 2>&1; then
    ADB="${ANDROID_HOME:-$HOME/Library/Android/sdk}/platform-tools/adb"
else
    ADB=$(command -v adb)
fi
MAESTRO="${MAESTRO:-$HOME/.maestro/bin/maestro}"
command -v maestro > /dev/null 2>&1 && MAESTRO=$(command -v maestro)

if [ "${RESET:-1}" = "1" ]; then
    echo "[run] clearing app state + re-pushing regtest config"
    "$ADB" shell pm clear com.stablechannels.app > /dev/null
    ./harness/push-test-config.sh
fi

"$MAESTRO" test "${@:-flows/}"
