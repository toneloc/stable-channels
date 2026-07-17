#!/usr/bin/env bash
# Prepare the iOS simulator for a clean run:
#   build the app (incremental) -> VERIFY the override code is actually in the
#   dylib (guards the stale-incremental-build trap) -> erase the sim -> install
#   -> push the regtest test_config.json. Leaves the app NOT launched so the
#   first flow's launchApp loads the overrides fresh.
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

PROJECT="$REPO_DIR/ios/StableChannels/StableChannels.xcodeproj"
SCHEME="StableChannels"
DD="$E2E_DIR/.dd-ios"                 # persistent DerivedData -> fast incremental
APP_GLOB="$DD/Build/Products/Debug-iphonesimulator/StableChannels.app"

command -v xcodebuild >/dev/null || die "xcodebuild not found (Xcode required for iOS)"

# The override code (TestOverrides.swift) lives in StableChannels.debug.dylib;
# the main binary is a thin stub. If this string is missing the build silently
# runs on MAINNET — see the 2026-07-16 incident.
dylib_has_overrides() {
    local dylib="$APP_GLOB/StableChannels.debug.dylib" n
    [ -f "$dylib" ] || dylib="$APP_GLOB/StableChannels"
    [ -f "$dylib" ] || return 1
    # grep -c (reads to EOF) not grep -q: with a 60MB dylib and pipefail,
    # grep -q exits on first match -> SIGPIPE kills strings -> pipeline
    # reports failure even on a match. Count instead.
    n="$(strings -a "$dylib" 2>/dev/null | grep -c 'test_config.json' || true)"
    [ "${n:-0}" -gt 0 ]
}

build_ios() {
    local mode="$1"   # build | "clean build"
    # shellcheck disable=SC2086
    xcodebuild -project "$PROJECT" -scheme "$SCHEME" -configuration Debug \
        -sdk iphonesimulator \
        -destination "platform=iOS Simulator,id=$IOS_SIM_UDID" \
        -derivedDataPath "$DD" $mode \
        >/tmp/sc-ios-build.log 2>&1 \
        || { tail -25 /tmp/sc-ios-build.log; die "xcodebuild failed ($mode)"; }
}

step "iOS app build"
info "incremental build → $DD"
build_ios build
if dylib_has_overrides; then
    ok "override code present in dylib"
else
    info "override code MISSING — forcing clean rebuild"
    build_ios "clean build"
    dylib_has_overrides || die "clean build still lacks TestOverrides — check pbxproj/DEBUG"
    ok "override code present after clean rebuild"
fi

step "iOS simulator ($IOS_SIM_UDID)"
info "erase + boot (clean wallet state) …"
xcrun simctl shutdown all >/dev/null 2>&1 || true
xcrun simctl erase "$IOS_SIM_UDID"
xcrun simctl boot "$IOS_SIM_UDID"
xcrun simctl bootstatus "$IOS_SIM_UDID" -b >/dev/null 2>&1 || true
ok "simulator booted"

info "install app …"
xcrun simctl install "$IOS_SIM_UDID" "$APP_GLOB"
ok "app installed"

info "push regtest config (before first launch) …"
( cd "$HARNESS_DIR" && HARNESS_HOST=localhost ./push-test-config-ios.sh >/dev/null )
ok "test_config.json written — device ready"
