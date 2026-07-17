#!/usr/bin/env bash
# Run the E2E flows on a prepared device with a descriptive, checkmarked
# scoreboard. State carries across flows (no clearState) — the device is reset
# once by prepare-*.sh, then 01 onboards, 02 trades on that state, etc.
#
# Usage: run-flows.sh <ios|android> [flow ...]     (flows default to canonical)
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

PLATFORM="${1:-}"; shift || true
[ "$PLATFORM" = "ios" ] || [ "$PLATFORM" = "android" ] || die "usage: run-flows.sh <ios|android> [flow ...]"

FLOWS=("$@"); [ "${#FLOWS[@]}" -gt 0 ] || FLOWS=("${CANONICAL_FLOWS[@]}")
PLAT_UP="$(printf '%s' "$PLATFORM" | tr '[:lower:]' '[:upper:]')"

# Pin maestro to the intended device (both a sim and an emulator may be booted).
if [ "$PLATFORM" = "ios" ]; then
    DEVICE="$IOS_SIM_UDID"
else
    DEVICE="$("$ADB" get-serialno 2>/dev/null)" || die "no android device (run prepare-android.sh)"
fi

# Clean price so a prior aborted flow-03 doesn't leave it at 101k.
harness_set_price 100000 || die "harness not reachable — is the backend up? (make up)"

step "Running ${#FLOWS[@]} flows on ${PLAT_UP}  (device $DEVICE)"

names=(); results=(); times=()
suite_start=$(date +%s)

for f in "${FLOWS[@]}"; do
    title="$(flow_title "$f")"; desc="$(flow_desc "$f")"
    printf '\n%s▶ %s%s\n' "$C_BOLD" "${title:-$f}" "$C_RESET"
    [ -n "$desc" ] && printf '  %s%s%s\n' "$C_DIM" "$desc" "$C_RESET"

    fstart=$(date +%s)
    if "$MAESTRO" test --device "$DEVICE" "$FLOWS_DIR/$f.yaml"; then
        rc=0; else rc=1; fi
    dur=$(( $(date +%s) - fstart ))

    names+=("$f"); times+=("$dur")
    if [ "$rc" -eq 0 ]; then results+=(pass); ok "$f  (${dur}s)"
    else results+=(fail); bad "$f  (FAILED after ${dur}s)"; fi
done

total=$(( $(date +%s) - suite_start ))
passed=0

step "Results — ${PLAT_UP}"
for i in "${!names[@]}"; do
    if [ "${results[$i]}" = "pass" ]; then
        passed=$((passed+1))
        printf '   %s✔%s %-26s %s(%ss)%s\n' "$C_GREEN" "$C_RESET" "${names[$i]}" "$C_DIM" "${times[$i]}" "$C_RESET"
    else
        printf '   %s✗%s %-26s %sFAILED%s\n' "$C_RED" "$C_RESET" "${names[$i]}" "$C_RED" "$C_RESET"
    fi
done

if [ "$passed" -eq "${#names[@]}" ]; then
    printf '\n%s✔ %d/%d passed in %dm%02ds%s\n' \
        "$C_GREEN$C_BOLD" "$passed" "${#names[@]}" "$((total/60))" "$((total%60))" "$C_RESET"
else
    printf '\n%s✗ %d/%d passed in %dm%02ds — see ~/.maestro/tests/ for screenshots%s\n' \
        "$C_RED$C_BOLD" "$passed" "${#names[@]}" "$((total/60))" "$((total%60))" "$C_RESET"
    exit 1
fi
