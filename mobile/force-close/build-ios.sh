#!/bin/bash
# Builds sc-force-close for the iOS app: sc_force_closeFFI.xcframework for the device and the
# simulator, and its Swift bindings, into the SCForceClose package in mobile/force-close/swift. Both
# are generated, not committed; run this before building the app.
#
# A dynamic framework, as ldk-node's LDKNodeFFI is a static Rust library and two static Rust
# libraries in one app define the same Rust standard library symbols.
set -euo pipefail

cd "$(dirname "$0")/../.."
PACKAGE=mobile/force-close/swift
NAME=sc_force_closeFFI
MIN_IOS=17.0

# The library reads the channel data that the app's ldk-node writes, so it must use the same LDK.
ldk_rev() { sed -n 's/^lightning = .*rev = "\([0-9a-f]*\)".*/\1/p' | sort -u; }
LDK_NODE_VERSION=$(grep -A2 'toneloc/ldk-node' ios/StableChannels/project.yml | sed -n 's/.*exactVersion: "\([0-9.]*\)".*/\1/p')
APP_LDK=$(curl -fsSL "https://raw.githubusercontent.com/toneloc/ldk-node/v$LDK_NODE_VERSION/Cargo.toml" | ldk_rev)
OUR_LDK=$(ldk_rev < mobile/force-close/Cargo.toml)
if [ -z "$APP_LDK" ] || [ "$APP_LDK" != "$OUR_LDK" ]; then
    echo "error: ldk-node $LDK_NODE_VERSION uses LDK '$APP_LDK', mobile/force-close uses '$OUR_LDK'" >&2
    exit 1
fi

export CARGO_PROFILE_RELEASE_OPT_LEVEL=z
export CARGO_PROFILE_RELEASE_LTO=true
export CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1
export CARGO_PROFILE_RELEASE_STRIP=true
export IPHONEOS_DEPLOYMENT_TARGET=$MIN_IOS

TARGETS="aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios"
rustup target add $TARGETS
for target in $TARGETS; do
    cargo build -p sc-force-close --release --target "$target" --locked
done

# The bindings come from a host build, which keeps the metadata the release builds strip.
BINDINGS=target/sc-force-close-swift
rm -rf "$BINDINGS"
cargo build -p sc-force-close --locked
cargo run -p sc-force-close --features bindgen --bin uniffi-bindgen --locked -- generate \
    --library target/debug/libsc_force_close.dylib --language swift --out-dir "$BINDINGS"

FRAMEWORKS=target/sc-force-close-frameworks
rm -rf "$FRAMEWORKS"
# framework <platform> <dylib>...
framework() {
    local platform=$1
    shift
    local dir="$FRAMEWORKS/$platform/$NAME.framework"
    mkdir -p "$dir/Headers" "$dir/Modules"
    lipo -create "$@" -output "$dir/$NAME"
    install_name_tool -id "@rpath/$NAME.framework/$NAME" "$dir/$NAME"
    cp "$BINDINGS/$NAME.h" "$dir/Headers/"
    printf 'framework module %s {\n    header "%s.h"\n    export *\n}\n' "$NAME" "$NAME" > "$dir/Modules/module.modulemap"
    cat > "$dir/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key><string>$NAME</string>
    <key>CFBundleIdentifier</key><string>com.stablechannels.scforceclose</string>
    <key>CFBundleName</key><string>$NAME</string>
    <key>CFBundlePackageType</key><string>FMWK</string>
    <key>CFBundleShortVersionString</key><string>1.0</string>
    <key>CFBundleVersion</key><string>1</string>
    <key>CFBundleSupportedPlatforms</key><array><string>$platform</string></array>
    <key>MinimumOSVersion</key><string>$MIN_IOS</string>
</dict>
</plist>
EOF
}
framework iPhoneOS target/aarch64-apple-ios/release/libsc_force_close.dylib
framework iPhoneSimulator target/aarch64-apple-ios-sim/release/libsc_force_close.dylib \
    target/x86_64-apple-ios/release/libsc_force_close.dylib

rm -rf "$PACKAGE/$NAME.xcframework"
xcodebuild -create-xcframework \
    -framework "$FRAMEWORKS/iPhoneOS/$NAME.framework" \
    -framework "$FRAMEWORKS/iPhoneSimulator/$NAME.framework" \
    -output "$PACKAGE/$NAME.xcframework"
mkdir -p "$PACKAGE/Sources/SCForceClose"
cp "$BINDINGS/sc_force_close.swift" "$PACKAGE/Sources/SCForceClose/"
