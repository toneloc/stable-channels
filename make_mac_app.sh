#!/usr/bin/env bash
# make_mac_app.sh – build universal Stable Channels.app (with icon if PNG present)
# -------------------------------------------------------------------------------
set -euo pipefail

# ─── config ──────────────────────────────────────────────────────────────
APP_NAME="Stable Channels"
APP_BUNDLE="${APP_NAME}.app"
IDENTIFIER="com.yourcompany.stablechannels"
VERSION="0.1.0"
BIN_NAME="stable-channels"
WRAPPER_NAME="$APP_NAME"

PNG_ICON="sc-icon-egui.png"     # 1024×1024 PNG in current dir
ICNS_NAME="AppIcon.icns"
# ─────────────────────────────────────────────────────────────────────────

echo "▶ Installing targets…"
rustup target add aarch64-apple-darwin x86_64-apple-darwin

echo "▶ Building release binaries…"
cargo build --release --bin "$BIN_NAME" --target aarch64-apple-darwin
cargo build --release --bin "$BIN_NAME" --target x86_64-apple-darwin

echo "▶ Creating universal binary…"
mkdir -p target/universal/release
lipo -create \
  -output "target/universal/release/${BIN_NAME}" \
  "target/aarch64-apple-darwin/release/${BIN_NAME}" \
  "target/x86_64-apple-darwin/release/${BIN_NAME}"

# ─── icon step (optional) ────────────────────────────────────────────────
ICON_PLIST_SNIPPET=""
if [[ -f "$PNG_ICON" ]]; then
  echo "▶ Converting $PNG_ICON → ${ICNS_NAME}…"
  TMP_DIR="$(mktemp -d)/icon.iconset"
  mkdir "$TMP_DIR"
  for sz in 16 32 64 128 256 512 1024; do
    sips -z "$sz" "$sz"   "$PNG_ICON" --out "$TMP_DIR/icon_${sz}x${sz}.png"        >/dev/null
    sips -z $((sz*2)) $((sz*2)) "$PNG_ICON" --out "$TMP_DIR/icon_${sz}x${sz}@2x.png" >/dev/null
  done
  iconutil --convert icns "$TMP_DIR" --output "${ICNS_NAME}"
  rm -rf "$TMP_DIR"
  ICON_PLIST_SNIPPET="
    <key>CFBundleIconFile</key> <string>${ICNS_NAME}</string>
    <key>CFBundleIconFiles</key> <array><string>${ICNS_NAME}</string></array>"
else
  echo "⚠️  $PNG_ICON not found – building without custom icon."
fi
# ─────────────────────────────────────────────────────────────────────────

echo "▶ Assembling .app bundle…"
rm -rf "$APP_BUNDLE"
mkdir -p "$APP_BUNDLE/Contents/MacOS" "$APP_BUNDLE/Contents/Resources"

# copy binaries
cp "target/universal/release/${BIN_NAME}" "$APP_BUNDLE/Contents/MacOS/"
cat >"$APP_BUNDLE/Contents/MacOS/${WRAPPER_NAME}" <<'SH'
#!/usr/bin/env bash
DIR="$(cd "$(dirname "$0")" && pwd)"
exec "$DIR/stable-channels" user
SH
chmod +x "$APP_BUNDLE/Contents/MacOS/${WRAPPER_NAME}"

# copy icon if one was produced
[[ -f "${ICNS_NAME}" ]] && cp "${ICNS_NAME}" "$APP_BUNDLE/Contents/Resources/${ICNS_NAME}"

# Info.plist
cat >"$APP_BUNDLE/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleName</key>           <string>${APP_NAME}</string>
  <key>CFBundleDisplayName</key>    <string>${APP_NAME}</string>
  <key>CFBundleIdentifier</key>     <string>${IDENTIFIER}</string>
  <key>CFBundleVersion</key>        <string>${VERSION}</string>
  <key>CFBundlePackageType</key>    <string>APPL</string>
  <key>CFBundleExecutable</key>     <string>${WRAPPER_NAME}</string>
  ${ICON_PLIST_SNIPPET}
  <key>LSMinimumSystemVersion</key> <string>10.13</string>
</dict></plist>
PLIST

echo "✅  ${APP_BUNDLE} built (universal; icon = ${ICNS_NAME:-none})."
