#!/usr/bin/env bash
# Run the E2E flows on a prepared device with a descriptive, checkmarked
# scoreboard. Stable Channels app data is reset once at suite startup, then
# state carries across flows: 01 onboards, 02 trades on that state, etc.
#
# Usage: run-flows.sh <ios|android> [flow ...]     (flows default to canonical)
# Set RESET_APP_STATE=0 only when intentionally continuing an existing run.
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

PLATFORM="${1:-}"; shift || true
[ "$PLATFORM" = "ios" ] || [ "$PLATFORM" = "android" ] || die "usage: run-flows.sh <ios|android> [flow ...]"

FLOWS=("$@"); [ "${#FLOWS[@]}" -gt 0 ] || FLOWS=("${CANONICAL_FLOWS[@]}")
PLAT_UP="$(printf '%s' "$PLATFORM" | tr '[:lower:]' '[:upper:]')"

# Pin maestro to the intended device (both a sim and an emulator may be booted).
if [ "$PLATFORM" = "ios" ]; then
    DEVICE="$(resolve_ios_sim_udid)"
else
    DEVICE="$("$ADB" get-serialno 2>/dev/null)" || die "no android device (run prepare-android.sh)"
fi

if [ "${RESET_APP_STATE:-1}" = "1" ]; then
    step "Reset Stable Channels app data"
    if [ "$PLATFORM" = "ios" ]; then
        IOS_APP_BUNDLE="$E2E_DIR/.dd-ios/Build/Products/Debug-iphonesimulator/StableChannels.app"
        [ -d "$IOS_APP_BUNDLE" ] || die "iOS app bundle missing; run prepare-ios.sh first"

        # Uninstalling clears the entire app container, including the wallet,
        # database, preferences, and cached state.
        xcrun simctl terminate "$DEVICE" "$APP_ID" >/dev/null 2>&1 || true
        if xcrun simctl get_app_container "$DEVICE" "$APP_ID" data >/dev/null 2>&1; then
            xcrun simctl uninstall "$DEVICE" "$APP_ID" >/dev/null \
                || die "could not uninstall $APP_ID from iOS simulator $DEVICE"
        fi
        xcrun simctl install "$DEVICE" "$IOS_APP_BUNDLE"
        (
            cd "$HARNESS_DIR"
            IOS_SIM_UDID="$DEVICE" HARNESS_HOST=localhost \
                ./push-test-config-ios.sh >/dev/null
        )
    else
        ANDROID_CLEAR_RESULT=""
        "$ADB" shell am force-stop "$APP_ID" >/dev/null 2>&1 || true
        ANDROID_CLEAR_RESULT="$("$ADB" shell pm clear "$APP_ID" 2>/dev/null | tr -d '\r')" \
            || die "could not clear Android app data for $APP_ID"
        [ "$ANDROID_CLEAR_RESULT" = "Success" ] \
            || die "Android app-data reset failed: ${ANDROID_CLEAR_RESULT:-no response}"
        ( cd "$HARNESS_DIR" && ./push-test-config.sh >/dev/null )
    fi
    ok "app data cleared and regtest config restored"

    # A clean suite begins at the canonical price. Flow 03 moves to $99,500
    # and intentionally leaves that price in place for every later flow.
    harness_set_price 100000 || die "harness not reachable — is the backend up? (make up)"
    ok "mock price reset to \$100,000"
else
    info "keeping existing app data (RESET_APP_STATE=0)"
    info "keeping the current mock price for the resumed lifecycle"
fi

step "Running ${#FLOWS[@]} flows on ${PLAT_UP}  (device $DEVICE)"

# Dim + indent maestro's own step lines so the suite's structure (badges,
# descriptions, verdicts) stays visually dominant; highlight failures.
# Piping also forces maestro into plain line output — stable everywhere.
style_maestro() {
    awk -v gray="$C_GRAY" -v red="$C_RED" -v grn="$C_GREEN" -v rst="$C_RESET" '
        function put(s) { print s; fflush() }           # stream through the tee pipe
        /FAILED|Exception|Assertion is false/ { put("   " red $0 rst); next }
        /\.\.\. COMPLETED$/ {
            sub(/\.\.\. COMPLETED$/, "");
            put("   " gray "· " $0 grn "✓" rst); next
        }
        /^[[:space:]]*$/ { next }                       # drop blank spacer lines
        /Debug tests faster|maestro cloud|^╭|^│|^╰/ { next }  # drop the ad box
        { put("   " gray $0 rst) }'
}

names=(); results=(); times=()
suite_start=$(date +%s)

for f in "${FLOWS[@]}"; do
    # "Step 7 — Onchain Send" -> badge "STEP 07" + bold title
    title="$(flow_title "$f")"; desc="$(flow_desc "$f")"
    num="${f%%_*}"
    pretty="${title#Step * — }"; [ "$pretty" = "$title" ] && pretty="${title:-$f}"

    printf '\n%s STEP %s %s %s%s%s\n' "$B_STEP" "$num" "$C_RESET" "$C_BOLD" "$pretty" "$C_RESET"
    [ -n "$desc" ] && printf '   %s%s%s\n' "$C_ITAL$C_GRAY" "$desc" "$C_RESET"

    fstart=$(date +%s)
    if "$MAESTRO" test --device "$DEVICE" "$FLOWS_DIR/$f.yaml" 2>&1 | style_maestro; then
        rc=0; else rc=1; fi
    dur=$(( $(date +%s) - fstart ))

    names+=("$f"); times+=("$dur")
    if [ "$rc" -eq 0 ]; then
        results+=(pass)
        printf '   %s ✔ PASS %s %s%s%s %s%ss%s\n' "$B_PASS" "$C_RESET" "$C_GREEN" "$f" "$C_RESET" "$C_GRAY" "$dur" "$C_RESET"
    else
        results+=(fail)
        printf '   %s ✗ FAIL %s %s%s%s %safter %ss · screenshots: ~/.maestro/tests/%s\n' \
            "$B_FAIL" "$C_RESET" "$C_RED$C_BOLD" "$f" "$C_RESET" "$C_GRAY" "$dur" "$C_RESET"
    fi
done

total=$(( $(date +%s) - suite_start ))
passed=0

step "Results — ${PLAT_UP}"
for i in "${!names[@]}"; do
    if [ "${results[$i]}" = "pass" ]; then
        passed=$((passed+1))
        printf '   %s✔%s %-26s %s%4ss%s\n' "$C_GREEN" "$C_RESET" "${names[$i]}" "$C_GRAY" "${times[$i]}" "$C_RESET"
    else
        printf '   %s✗ %-26s %sFAILED%s\n' "$C_RED$C_BOLD" "${names[$i]}" "$C_RED" "$C_RESET"
    fi
done

if [ "$passed" -eq "${#names[@]}" ]; then
    printf '\n%s ✔ %d/%d PASSED %s %sin %dm%02ds%s\n' \
        "$B_PASS" "$passed" "${#names[@]}" "$C_RESET" "$C_GRAY" "$((total/60))" "$((total%60))" "$C_RESET"
else
    printf '\n%s ✗ %d/%d PASSED %s %sin %dm%02ds — see ~/.maestro/tests/ for screenshots%s\n' \
        "$B_FAIL" "$passed" "${#names[@]}" "$C_RESET" "$C_GRAY" "$((total/60))" "$((total%60))" "$C_RESET"
    exit 1
fi
