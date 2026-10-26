#!/usr/bin/env bash
# Builds "Codex Resets.app" into dist/: the release binary, the icon, the
# Info.plist (LSUIElement, so it is a menu bar app) and an ad-hoc signature.
#
#   scripts/package.sh          build the bundle
#   scripts/package.sh --run    build it and open it

set -euo pipefail

cd "$(dirname "$0")/.."

APP="dist/Codex Resets.app"
BIN="target/release/codex-reset"

echo "==> drawing the icon"
swift scripts/make-icon.swift build/AppIcon.iconset
iconutil -c icns build/AppIcon.iconset -o build/AppIcon.icns

echo "==> building the release binary"
cargo build --release

echo "==> assembling $APP"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/CodexResets"
cp resources/Info.plist "$APP/Contents/Info.plist"
cp build/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"

# Local ad-hoc signature; this does not perform Developer ID signing or notarization.
codesign --force --sign - "$APP" >/dev/null
echo "==> built $APP"

if [ "${1:-}" = "--run" ]; then
  open "$APP"
fi
