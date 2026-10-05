#!/usr/bin/env bash
# Prepare the Android emulator for a clean run:
#   boot the emulator if needed -> install the debug APK -> clear app data ->
#   push the regtest test_config.json. Leaves the app freshly reset.
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

step "Android emulator"
if ! "$ADB" get-state >/dev/null 2>&1; then
    [ -x "$EMULATOR" ] || die "emulator binary not found at $EMULATOR"
    info "booting AVD $ANDROID_AVD …"
    nohup "$EMULATOR" -avd "$ANDROID_AVD" -no-snapshot-load \
        -netdelay none -netspeed full >/tmp/sc-emu.log 2>&1 &
fi
info "waiting for device + boot completion …"
"$ADB" wait-for-device
until [ "$("$ADB" shell getprop sys.boot_completed 2>/dev/null | tr -d '\r')" = "1" ]; do
    sleep 2
done
ok "emulator booted ($("$ADB" get-serialno))"

# Kill the IME "Try out your stylus" onboarding sheet: it pops over any focused
# text field (the invoice/address inputs in flows 06/07/12), swallows input and
# occludes buttons like "Send Max". Disabling stylus handwriting stops it.
"$ADB" shell settings put secure stylus_handwriting_enabled 0 >/dev/null 2>&1 || true
ok "stylus-handwriting onboarding disabled"

step "Android app build"
info "./gradlew installDebug …"
( cd "$REPO_DIR/android" && ./gradlew installDebug ) >/tmp/sc-android-build.log 2>&1 \
    || { tail -25 /tmp/sc-android-build.log; die "installDebug failed"; }
ok "debug APK installed"

step "Reset app state"
info "pm clear + push regtest config …"
"$ADB" shell pm clear "$APP_ID" >/dev/null
( cd "$HARNESS_DIR" && ./push-test-config.sh >/dev/null )
ok "app data cleared, test_config.json pushed — device ready"
