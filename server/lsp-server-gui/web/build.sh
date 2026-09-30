#!/usr/bin/env bash
# Builds the static web (WASM) bundle into server/lsp-server-gui/dist.
set -euo pipefail

crate_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
repo_root="$(cd "$crate_dir/../.." && pwd)"
target_dir="${CARGO_TARGET_DIR:-$repo_root/target}"
dist="$crate_dir/dist"

# wasm-bindgen-cli must match the wasm-bindgen crate version pinned in Cargo.lock.
locked="$(awk '/^name = "wasm-bindgen"$/ { getline; gsub(/"/, "", $3); print $3; exit }' "$repo_root/Cargo.lock")"
installed="$(wasm-bindgen --version 2>/dev/null | awk '{ print $2 }')"
if [ "$locked" != "$installed" ]; then
	echo "wasm-bindgen-cli $locked is required (found '${installed:-none}')." >&2
	echo "Install it with: cargo install --locked wasm-bindgen-cli --version $locked" >&2
	exit 1
fi

# Size-optimized release for the browser; scoped to this build, the workspace profile is untouched.
export CARGO_PROFILE_RELEASE_OPT_LEVEL="${CARGO_PROFILE_RELEASE_OPT_LEVEL:-z}"
export CARGO_PROFILE_RELEASE_LTO="${CARGO_PROFILE_RELEASE_LTO:-true}"
export CARGO_PROFILE_RELEASE_CODEGEN_UNITS="${CARGO_PROFILE_RELEASE_CODEGEN_UNITS:-1}"

cargo build --locked -p lsp-server-gui --release --target wasm32-unknown-unknown \
	--no-default-features --features web --manifest-path "$repo_root/Cargo.toml"

rm -rf "$dist"
mkdir -p "$dist"
wasm-bindgen --target web --no-typescript --out-name lsp_server_gui --out-dir "$dist" \
	"$target_dir/wasm32-unknown-unknown/release/lsp-server-gui.wasm"

if command -v wasm-opt >/dev/null 2>&1; then
	if ! wasm-opt -O2 --enable-bulk-memory --enable-nontrapping-float-to-int --enable-sign-ext \
		--enable-reference-types --enable-multivalue --enable-mutable-globals \
		"$dist/lsp_server_gui_bg.wasm" -o "$dist/lsp_server_gui_bg.opt.wasm"; then
		echo "warning: wasm-opt failed; shipping the unoptimized module" >&2
		rm -f "$dist/lsp_server_gui_bg.opt.wasm"
	else
		mv "$dist/lsp_server_gui_bg.opt.wasm" "$dist/lsp_server_gui_bg.wasm"
	fi
else
	echo "warning: wasm-opt not found; shipping the unoptimized module" >&2
fi

# Cache-bust the module URLs with the wasm content hash.
hash="$(sha256sum "$dist/lsp_server_gui_bg.wasm" | cut -c1-16)"
sed "s/__BUILD_HASH__/$hash/g" "$crate_dir/web/index.html" > "$dist/index.html"
echo "Web bundle written to $dist ($(du -h "$dist/lsp_server_gui_bg.wasm" | cut -f1) wasm)"
