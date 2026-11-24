#!/usr/bin/env bash
# Build lsp-server-gui as a proper macOS .app bundle so it gets the Stable Channels
# icon in the Dock/Launchpad instead of the generic "exec" placeholder shown for a
# bare Mach-O binary. macOS ignores tao's set_window_icon on this platform, so the
# icon has to come from an Info.plist + .icns inside a real bundle — there's no
# runtime API for it.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."
PROFILE="${1:-release}"

# Cargo reports the actual executable path, including custom target directories,
# target triples, and profile directory names (for example, dev uses debug).
BIN="$(cargo build --profile "$PROFILE" --features native --bin lsp-server-gui \
	--message-format=json-render-diagnostics | python3 -c '
import json
import os
import sys

executable = None
for line in sys.stdin:
    message = json.loads(line)
    target = message.get("target", {})
    if (message.get("reason") == "compiler-artifact"
            and target.get("name") == "lsp-server-gui"
            and "bin" in target.get("kind", [])
            and message.get("executable")):
        executable = message["executable"]
if executable is None:
    sys.exit("Cargo did not report the lsp-server-gui executable")
print(os.path.abspath(executable))
')"

APP="$(dirname "$BIN")/bundle/macos/LSP Server GUI.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

cp "$BIN" "$APP/Contents/MacOS/lsp-server-gui"
cp macos/Info.plist "$APP/Contents/Info.plist"
cp macos/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"

# Ad-hoc sign so Launch Services and Gatekeeper treat it as a normal local app
# (unsigned bundles can otherwise keep showing a stale/generic icon in Finder).
codesign --force --deep --sign - "$APP" >/dev/null 2>&1 || true

echo "Built: $APP"
echo "Run with: open \"$APP\""
