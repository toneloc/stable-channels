#!/usr/bin/env bash
# Build lsp-server-gui as a proper macOS .app bundle so it gets the Stable Channels
# icon in the Dock/Launchpad instead of the generic "exec" placeholder shown for a
# bare Mach-O binary. macOS ignores tao's set_window_icon on this platform, so the
# icon has to come from an Info.plist + .icns inside a real bundle — there's no
# runtime API for it.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."
PROFILE="${1:-release}"

cargo build --profile "$PROFILE" --features native

case "$PROFILE" in
	release) BIN_DIR="../../target/release" ;;
	dev|debug) BIN_DIR="../../target/debug" ;;
	*) BIN_DIR="../../target/$PROFILE" ;;
esac

APP="../../target/$PROFILE/bundle/macos/LSP Server GUI.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

cp "$BIN_DIR/lsp-server-gui" "$APP/Contents/MacOS/lsp-server-gui"
cp macos/Info.plist "$APP/Contents/Info.plist"
cp macos/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"

# Ad-hoc sign so Launch Services and Gatekeeper treat it as a normal local app
# (unsigned bundles can otherwise keep showing a stale/generic icon in Finder).
codesign --force --deep --sign - "$APP" >/dev/null 2>&1 || true

echo "Built: $APP"
echo "Run with: open \"$APP\""
