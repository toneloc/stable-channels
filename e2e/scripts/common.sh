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
# Colors are ON by default: make pipes everything through `tee .last-run.log`,
# so a tty check would strip color exactly where people watch (terminal and
# `tail -f`). Opt out with NO_COLOR=1 (or TERM=dumb).
if [ -z "${NO_COLOR:-}" ] && [ "${TERM:-}" != "dumb" ]; then
    C_DIM=$'\033[2m';  C_BOLD=$'\033[1m';   C_ITAL=$'\033[3m'
    C_GREEN=$'\033[32m'; C_RED=$'\033[31m'; C_CYAN=$'\033[36m'
    C_YELLOW=$'\033[33m'; C_GRAY=$'\033[38;5;245m'
    # chips: white text on colored background
    B_STEP=$'\033[48;5;24;38;5;255;1m'   # deep teal-blue — flow badges
    B_PASS=$'\033[48;5;22;38;5;255;1m'   # green — pass chip
    B_FAIL=$'\033[48;5;88;38;5;255;1m'   # red — fail chip
    C_RESET=$'\033[0m'
else
    C_DIM=""; C_BOLD=""; C_ITAL=""; C_GREEN=""; C_RED=""; C_CYAN=""
    C_YELLOW=""; C_GRAY=""; B_STEP=""; B_PASS=""; B_FAIL=""; C_RESET=""
fi
say()  { printf '%s\n' "$*"; }
info() { printf '%s•%s %s\n' "$C_GRAY" "$C_RESET" "$*"; }
step() { printf '\n%s━━━%s %s%s%s %s━━━%s\n' "$C_GRAY" "$C_RESET" "$C_BOLD$C_CYAN" "$*" "$C_RESET" "$C_GRAY" "$C_RESET"; }
ok()   { printf '   %s✔%s %s\n' "$C_GREEN" "$C_RESET" "$*"; }
bad()  { printf '   %s✗%s %s\n' "$C_RED" "$C_RESET" "$*"; }
die()  { printf '%serror:%s %s\n' "$C_RED$C_BOLD" "$C_RESET" "$*" >&2; exit 1; }

# --- helpers ----------------------------------------------------------------

sc_bytes_from_gib() {
    local gib="${1:?GiB value required}"
    case "$gib" in
        ''|*[!0-9]*) die "invalid GiB value: $gib" ;;
    esac
    printf '%s\n' "$((gib * 1024 * 1024 * 1024))"
}

sc_human_bytes() {
    awk -v bytes="${1:-0}" 'BEGIN {
        split("B KiB MiB GiB TiB", unit)
        i = 1
        while (bytes >= 1024 && i < 5) {
            bytes /= 1024
            i++
        }
        printf "%.1f %s", bytes, unit[i]
    }'
}

sc_existing_df_path() {
    local path="${1:?path required}"
    while [ ! -e "$path" ]; do
        local parent
        parent="$(dirname "$path")"
        if [ "$parent" = "$path" ]; then
            path="/"
            break
        fi
        path="$parent"
    done
    printf '%s\n' "$path"
}

sc_available_bytes() {
    local path
    path="$(sc_existing_df_path "${1:?path required}")"
    df -Pk "$path" | awk 'NR == 2 { printf "%.0f\n", $4 * 1024 }'
}

sc_path_bytes() {
    local path="${1:?path required}"
    if [ ! -e "$path" ]; then
        printf '0\n'
        return 0
    fi
    du -sk "$path" 2>/dev/null | awk 'NR == 1 { printf "%.0f\n", $1 * 1024 }'
}

sc_require_free_space() {
    local path="${1:-$REPO_DIR}"
    local min_gib="${2:-${SC_MIN_FREE_GIB:-25}}"
    local label="${3:-operation}"
    local available required

    required="$(sc_bytes_from_gib "$min_gib")"
    available="$(sc_available_bytes "$path")"
    [ -n "$available" ] || die "could not determine free space near $path"
    if [ "$available" -lt "$required" ]; then
        die "$label needs at least ${min_gib} GiB free near $path; only $(sc_human_bytes "$available") is free. Run cleanup before retrying."
    fi

    info "$label disk preflight: $(sc_human_bytes "$available") free (min ${min_gib} GiB)"
}

sc_warn_docker_raw_size() {
    local warn_gib="${1:-${SC_DOCKER_RAW_WARN_GIB:-150}}"
    [ "${SC_SKIP_DOCKER_RAW_WARN:-0}" = "1" ] && return 0

    local raw="$HOME/Library/Containers/com.docker.docker/Data/vms/0/data/Docker.raw"
    [ -f "$raw" ] || return 0

    local actual warn_bytes
    actual="$(sc_path_bytes "$raw")"
    [ -n "$actual" ] || return 0
    warn_bytes="$(sc_bytes_from_gib "$warn_gib")"
    if [ "$actual" -ge "$warn_bytes" ]; then
        info "Docker.raw is $(sc_human_bytes "$actual") (warn threshold ${warn_gib} GiB)"
        info "Docker Desktop may keep freed space until you prune/compact it; use 'docker system prune -a --volumes' only when you are ready to delete unused Docker state."
    fi
}

sc_guard_path_size() {
    local path="${1:?path required}"
    local max_gib="${2:?max GiB required}"
    local label="${3:-$path}"
    local reset_env="${4:-}"

    [ -e "$path" ] || return 0

    local size max_bytes
    size="$(sc_path_bytes "$path")"
    [ -n "$size" ] || die "could not determine size for $path"
    max_bytes="$(sc_bytes_from_gib "$max_gib")"
    if [ "$size" -le "$max_bytes" ]; then
        info "$label size: $(sc_human_bytes "$size") (max ${max_gib} GiB)"
        return 0
    fi

    if [ -n "$reset_env" ]; then
        local reset_value="${!reset_env:-0}"
        if [ "$reset_value" = "1" ]; then
            info "$label is $(sc_human_bytes "$size") (max ${max_gib} GiB); removing because $reset_env=1"
            rm -rf "$path"
            return 0
        fi
        die "$label is $(sc_human_bytes "$size") (max ${max_gib} GiB). Re-run with $reset_env=1 to wipe it, or remove $path manually."
    fi

    die "$label is $(sc_human_bytes "$size") (max ${max_gib} GiB). Remove it before retrying."
}

sc_positive_int() {
    case "${1:-}" in
        ''|*[!0-9]*) return 1 ;;
        *) return 0 ;;
    esac
}

sc_command_with_timeout() {
    local seconds="${1:?timeout seconds required}"
    shift
    if command -v perl >/dev/null 2>&1; then
        perl -e 'my $seconds = shift @ARGV; alarm $seconds; exec @ARGV' "$seconds" "$@"
    else
        "$@"
    fi
}

sc_docker_or_host_mem_bytes() {
    local bytes

    if command -v docker >/dev/null 2>&1; then
        bytes="$(sc_command_with_timeout 5 docker info --format '{{.MemTotal}}' 2>/dev/null || true)"
        if sc_positive_int "$bytes" && [ "$bytes" -gt 0 ]; then
            printf 'docker:%s\n' "$bytes"
            return 0
        fi
    fi

    if command -v sysctl >/dev/null 2>&1; then
        bytes="$(sysctl -n hw.memsize 2>/dev/null || true)"
        if sc_positive_int "$bytes" && [ "$bytes" -gt 0 ]; then
            printf 'host:%s\n' "$bytes"
            return 0
        fi
    fi

    return 1
}

sc_autotune_docker_build_limits() {
    local compose_was_set="${COMPOSE_PARALLEL_LIMIT+x}"
    local jobs_was_set="${SC_DOCKER_CARGO_JOBS+x}"
    local codegen_was_set="${SC_DOCKER_CODEGEN_UNITS+x}"
    local compose=1 jobs=1 codegen=1 source="unknown" bytes="" gib=0
    local detected

    detected="$(sc_docker_or_host_mem_bytes || true)"
    if [ -n "$detected" ]; then
        source="${detected%%:*}"
        bytes="${detected#*:}"
        gib="$((bytes / 1024 / 1024 / 1024))"

        if [ "$source" = "docker" ]; then
            if [ "$gib" -ge 48 ]; then
                compose=3; jobs=6; codegen=8
            elif [ "$gib" -ge 24 ]; then
                compose=2; jobs=4; codegen=4
            elif [ "$gib" -ge 12 ]; then
                compose=1; jobs=2; codegen=2
            fi
        else
            # Host RAM is only a fallback; Docker Desktop may still be capped.
            if [ "$gib" -ge 48 ]; then
                compose=1; jobs=3; codegen=3
            elif [ "$gib" -ge 24 ]; then
                compose=1; jobs=2; codegen=2
            fi
        fi
    fi

    [ -n "${COMPOSE_PARALLEL_LIMIT:-}" ] || export COMPOSE_PARALLEL_LIMIT="$compose"
    [ -n "${SC_DOCKER_CARGO_JOBS:-}" ] || export SC_DOCKER_CARGO_JOBS="$jobs"
    [ -n "${SC_DOCKER_CODEGEN_UNITS:-}" ] || export SC_DOCKER_CODEGEN_UNITS="$codegen"

    # Compose >=2.34 defaults to building every service in one parallel BuildKit
    # "bake" graph, which IGNORES COMPOSE_PARALLEL_LIMIT — all Rust images then
    # compile at once and the concurrent rustc peaks OOM-kill the build on a
    # memory-capped Docker VM (SIGKILL / ResourceExhausted). When we want serial
    # builds, turn bake off so compose builds one service at a time.
    if [ "$COMPOSE_PARALLEL_LIMIT" = "1" ]; then
        [ -n "${COMPOSE_BAKE:-}" ] || export COMPOSE_BAKE=false
    fi

    local detail="memory unknown; safe defaults"
    if [ -n "$bytes" ]; then
        detail="$(sc_human_bytes "$bytes") $source RAM"
    fi
    local override_note=""
    if [ -n "$compose_was_set" ] || [ -n "$jobs_was_set" ] || [ -n "$codegen_was_set" ]; then
        override_note="; manual overrides honored"
    fi
    info "docker build limits: compose parallel=$COMPOSE_PARALLEL_LIMIT, cargo jobs=$SC_DOCKER_CARGO_JOBS, codegen units=$SC_DOCKER_CODEGEN_UNITS ($detail$override_note)"
}

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
