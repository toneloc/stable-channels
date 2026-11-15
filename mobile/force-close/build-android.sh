#!/bin/bash
# Builds sc-force-close for the Android app: libsc_force_close.so for the ABIs ldk-node-android
# ships, into android/app/src/main/jniLibs, and its Kotlin bindings. Both are generated, not
# committed; run this before building the app.
#
# Needs Rust and the Android NDK (ANDROID_NDK_ROOT or ANDROID_NDK_HOME, otherwise the newest NDK of
# the Android SDK).
set -euo pipefail

cd "$(dirname "$0")/../.."
APP=android/app/src/main

# The library reads the channel data that the app's ldk-node writes, so it must use the same LDK.
ldk_rev() { sed -n 's/^lightning = .*rev = "\([0-9a-f]*\)".*/\1/p' | sort -u; }
LDK_NODE_VERSION=$(sed -n 's/.*ldk-node-android:\([0-9.]*\)@aar.*/\1/p' android/app/build.gradle.kts)
APP_LDK=$(curl -fsSL "https://raw.githubusercontent.com/toneloc/ldk-node/v$LDK_NODE_VERSION/Cargo.toml" | ldk_rev)
OUR_LDK=$(ldk_rev < mobile/force-close/Cargo.toml)
if [ -z "$APP_LDK" ] || [ "$APP_LDK" != "$OUR_LDK" ]; then
    echo "error: ldk-node $LDK_NODE_VERSION uses LDK '$APP_LDK', mobile/force-close uses '$OUR_LDK'" >&2
    exit 1
fi

SDK="${ANDROID_HOME:-$HOME/Library/Android/sdk}"
NDK="${ANDROID_NDK_ROOT:-${ANDROID_NDK_HOME:-$(ls -d "$SDK"/ndk/* | sort -V | tail -1)}}"
case "$OSTYPE" in
    darwin*) HOST=darwin-x86_64 HOST_LIB=dylib ;;
    *) HOST=linux-x86_64 HOST_LIB=so ;;
esac
CLANG_DIR="$NDK/toolchains/llvm/prebuilt/$HOST/bin"

# Small, and 16 KB page aligned as Play requires, like ldk-node-android.
export CARGO_PROFILE_RELEASE_OPT_LEVEL=z
export CARGO_PROFILE_RELEASE_LTO=true
export CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1
export CARGO_PROFILE_RELEASE_STRIP=true

rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android
for abi_target in \
    arm64-v8a:aarch64-linux-android:aarch64-linux-android26-clang \
    armeabi-v7a:armv7-linux-androideabi:armv7a-linux-androideabi26-clang \
    x86_64:x86_64-linux-android:x86_64-linux-android26-clang; do
    IFS=: read -r abi target clang <<<"$abi_target"
    target_var=$(echo "$target" | tr 'a-z-' 'A-Z_')
    env "CARGO_TARGET_${target_var}_LINKER=$CLANG_DIR/$clang" \
        "CARGO_TARGET_${target_var}_RUSTFLAGS=-C link-args=-Wl,-z,max-page-size=16384,-z,common-page-size=16384" \
        "CC_${target//-/_}=$CLANG_DIR/$clang" \
        "AR_${target//-/_}=$CLANG_DIR/llvm-ar" \
        cargo build -p sc-force-close --release --target "$target" --locked
    mkdir -p "$APP/jniLibs/$abi"
    cp "target/$target/release/libsc_force_close.so" "$APP/jniLibs/$abi/"
done

# The bindings come from a host build, which keeps the metadata the release builds strip.
cargo build -p sc-force-close --locked
cargo run -p sc-force-close --features bindgen --bin uniffi-bindgen --locked -- generate \
    --library "target/debug/libsc_force_close.$HOST_LIB" --language kotlin --no-format \
    --out-dir "$APP/java"
