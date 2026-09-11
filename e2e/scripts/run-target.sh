#!/usr/bin/env bash
# Hold one lock across backend preparation and the complete test lifecycle.
# The simulator, mock price, and Docker services are shared mutable resources;
# overlapping runs otherwise erase devices or restart the LSP during a flow.
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

target="${1:-}"
shift || true

case "$target" in
    ios|android|mac-smoke|mac-flows|mac-ui|mac-demo|mac-all|up|rebuild|down|clean) ;;
    *) die "usage: run-target.sh <ios|android|mac-smoke|mac-flows|mac-ui|mac-demo|mac-all|up|rebuild|down|clean> [flows ...]" ;;
esac

if [ "${SC_E2E_RUN_LOCK_HELD:-}" != "$SC_E2E_RUN_LOCK_FILE" ]; then
    sc_with_e2e_run_lock "$target" "$0" "$target" "$@"
    exit $?
fi

case "$target" in
    ios|android)
        "$E2E_DIR/scripts/backend.sh"
        "$E2E_DIR/scripts/prepare-$target.sh"
        "$E2E_DIR/scripts/run-flows.sh" "$target" "$@"
        ;;
    mac-smoke)
        "$E2E_DIR/scripts/backend.sh"
        "$E2E_DIR/scripts/run-mac-smoke.sh"
        ;;
    mac-flows)
        "$E2E_DIR/scripts/backend.sh"
        "$E2E_DIR/scripts/run-mac-flows.sh"
        ;;
    mac-ui)
        "$E2E_DIR/scripts/backend.sh"
        "$E2E_DIR/scripts/run-mac-ui.sh"
        ;;
    mac-demo)
        "$E2E_DIR/scripts/backend.sh"
        "$E2E_DIR/scripts/run-mac-demo.sh"
        ;;
    mac-all)
        "$E2E_DIR/scripts/backend.sh"
        "$E2E_DIR/scripts/run-mac-smoke.sh"
        "$E2E_DIR/scripts/run-mac-flows.sh"
        ;;
    up)
        "$E2E_DIR/scripts/backend.sh"
        ;;
    rebuild)
        REBUILD=1 "$E2E_DIR/scripts/backend.sh"
        ;;
    down)
        (cd "$HARNESS_DIR" && docker compose --profile gui down --remove-orphans)
        ;;
    clean)
        (cd "$HARNESS_DIR" && docker compose --profile gui down -v --remove-orphans)
        ;;
esac
