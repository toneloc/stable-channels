#!/usr/bin/env bash
# Cargo runner (see ../.cargo/config.toml): wraps the freshly built binary in a
# minimal macOS .app bundle and execs it from inside the bundle, so `cargo run`
# shows the Stable Channels icon in the Dock instead of the generic "exec"
# placeholder macOS uses for a bare Mach-O binary. macOS has no runtime API for
# setting a process's Dock icon (tao's set_window_icon is a no-op here) — the
# icon can only come from an Info.plist + .icns next to the executable, which
# is also why we exec the binary at its path *inside* the bundle rather than
# `open` the bundle (stdout/stderr/exit code need to stay attached to the
# terminal cargo run is running in).
set -euo pipefail

BIN="$1"
shift

# This runner is configured workspace-wide (so `cargo run` works from any
# directory), but only lsp-server-gui wants the bundling treatment — every
# other workspace binary just runs straight through, unchanged.
if [[ "$(basename "$BIN")" != "lsp-server-gui" ]]; then
	exec "$BIN" "$@"
fi

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROFILE_DIR="$(dirname "$BIN")"
APP="$PROFILE_DIR/bundle/macos/LSP Server GUI.app"

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/lsp-server-gui"
cp "$HERE/Info.plist" "$APP/Contents/Info.plist"
cp "$HERE/AppIcon.icns" "$APP/Contents/Resources/AppIcon.icns"
codesign --force --sign - "$APP" >/dev/null 2>&1 || true

exec "$APP/Contents/MacOS/lsp-server-gui" "$@"
