#!/usr/bin/env bash
# Shared helpers for the E2E runner scripts (backend.sh, prepare-*.sh,
# run-flows.sh). Source this; don't execute it.
set -euo pipefail

# --- paths ------------------------------------------------------------------
E2E_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPO_DIR="$(cd "$E2E_DIR/.." && pwd)"
HARNESS_DIR="$E2E_DIR/harness"
FLOWS_DIR="$E2E_DIR/flows"

APP_ID="com.stablechannels.app"
HARNESS_API="http://localhost:9737"

# The canonical lifecycle (10/11 backup+import excluded — nav TODO). Override
# by passing an explicit flow list to run-flows.sh.
CANONICAL_FLOWS=(
    01_onboard_lightning 02_btc_to_usd 03_usd_stability 04_lightning_receive
    05_onchain_receive 06_lightning_send 07_onchain_send 08_usd_to_btc
    09_close_channel 12_offboard_onchain
)

# iOS simulator selection is resolved lazily by iOS scripts so Android targets
# can source this file on machines without Xcode. Set IOS_SIM_UDID to pin a
# device exactly, or IOS_SIM_NAME to choose by simulator name.

# --- tool resolution --------------------------------------------------------
ADB="$(command -v adb || echo "${ANDROID_HOME:-$HOME/Library/Android/sdk}/platform-tools/adb")"
EMULATOR="${ANDROID_HOME:-$HOME/Library/Android/sdk}/emulator/emulator"
MAESTRO="$(command -v maestro || echo "$HOME/.maestro/bin/maestro")"
ANDROID_AVD="${ANDROID_AVD:-Medium_Phone_API_36.1}"

# --- pretty output ----------------------------------------------------------
if [ -t 1 ]; then
    C_DIM=$'\033[2m'; C_BOLD=$'\033[1m'; C_GREEN=$'\033[32m'
    C_RED=$'\033[31m'; C_CYAN=$'\033[36m'; C_RESET=$'\033[0m'
else
    C_DIM=""; C_BOLD=""; C_GREEN=""; C_RED=""; C_CYAN=""; C_RESET=""
fi
say()  { printf '%s\n' "$*"; }
info() { printf '%s•%s %s\n' "$C_DIM" "$C_RESET" "$*"; }
step() { printf '\n%s━━━ %s ━━━%s\n' "$C_BOLD$C_CYAN" "$*" "$C_RESET"; }
ok()   { printf '   %s✔%s %s\n' "$C_GREEN" "$C_RESET" "$*"; }
bad()  { printf '   %s✗%s %s\n' "$C_RED" "$C_RESET" "$*"; }
die()  { printf '%serror:%s %s\n' "$C_RED$C_BOLD" "$C_RESET" "$*" >&2; exit 1; }

# --- helpers ----------------------------------------------------------------

extract_first_sim_udid() {
    sed -nE 's/.*\(([0-9A-Fa-f]{8}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{12})\).*/\1/p' \
        | head -n 1
}

resolve_ios_sim_udid() {
    if [ -n "${IOS_SIM_UDID:-}" ]; then
        printf '%s\n' "$IOS_SIM_UDID"
        return 0
    fi

    command -v xcrun >/dev/null || die "xcrun not found (Xcode required for iOS); set IOS_SIM_UDID to a simulator UDID"

    local devices requested udid
    devices="$(xcrun simctl list devices available 2>/dev/null)" \
        || die "could not list iOS simulators; set IOS_SIM_UDID to a simulator UDID"
    requested="${IOS_SIM_NAME:-}"

    if [ -n "$requested" ]; then
        udid="$(printf '%s\n' "$devices" | grep -F "$requested" | extract_first_sim_udid || true)"
        [ -n "$udid" ] || die "no available iOS simulator matched IOS_SIM_NAME=$requested; set IOS_SIM_UDID explicitly"
        printf '%s\n' "$udid"
        return 0
    fi

    udid="$(printf '%s\n' "$devices" | grep -E 'iPhone.*\(Booted\)' | extract_first_sim_udid || true)"
    if [ -z "$udid" ]; then
        udid="$(printf '%s\n' "$devices" | grep -E 'iPhone' | extract_first_sim_udid || true)"
    fi

    [ -n "$udid" ] || die "no available iPhone simulator found; create one or set IOS_SIM_UDID explicitly"
    printf '%s\n' "$udid"
}

# Read the LSP (ldk-server) node id straight from its startup log line:
#   "Starting up LDK Node with node ID <66-hex> on network: regtest"
lsp_node_id_from_logs() {
    (cd "$HARNESS_DIR" && docker compose logs ldk-server 2>/dev/null) \
        | grep -oE 'node ID [0-9a-f]{66}' | tail -1 | awk '{print $3}'
}

# Wait until ldk-server has printed the node id that the harness must connect to.
wait_for_lsp_node_id() {
    local live
    for _ in $(seq 1 60); do
        live="$(lsp_node_id_from_logs || true)"
        if [ -n "$live" ]; then
            printf '%s\n' "$live"
            return 0
        fi
        sleep 2
    done
    return 1
}

# Ensure harness/.env's LSP_NODE_ID matches the running ldk-server. Rewrites it
# if a fresh volume gave the node a new id.
sync_lsp_node_id() {
    local env_file="$HARNESS_DIR/.env" live current
    SYNC_LSP_NODE_ID_UPDATED=0
    live="${1:-}"
    [ -n "$live" ] || live="$(lsp_node_id_from_logs || true)"
    [ -n "$live" ] || { info "ldk-server node id not in logs yet"; return 0; }
    current="$(grep -sE '^LSP_NODE_ID=' "$env_file" | cut -d= -f2 || true)"
    if [ "$current" != "$live" ]; then
        info "updating LSP_NODE_ID -> ${live:0:16}..."
        if [ -f "$env_file" ] && grep -qE '^LSP_NODE_ID=' "$env_file"; then
            # portable in-place edit (no gnu sed dependency)
            grep -v '^LSP_NODE_ID=' "$env_file" > "$env_file.tmp" || true
            mv "$env_file.tmp" "$env_file"
        fi
        printf 'LSP_NODE_ID=%s\n' "$live" >> "$env_file"
        SYNC_LSP_NODE_ID_UPDATED=1
    fi
}

# Set the mocked BTC/USD price on the harness (drives BOTH app and LSP feeds).
harness_set_price() {
    curl -fsS -X POST "$HARNESS_API/price" \
        -H 'Content-Type: application/json' -d "{\"price\":${1:-100000}}" >/dev/null
}

# True once the harness reports a ready channel to the LSP.
harness_channel_ready() {
    curl -fsS "$HARNESS_API/info" 2>/dev/null \
        | grep -q '"ready":true' 2>/dev/null
}

# Human title + one-line description parsed from a flow's header comments.
flow_title() { sed -n 's/^# //p' "$FLOWS_DIR/$1.yaml" | sed -n '1p'; }
flow_desc()  { sed -n 's/^# //p' "$FLOWS_DIR/$1.yaml" | sed -n '2p' | tr -d '"'; }
