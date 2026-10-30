#!/usr/bin/env bash
# Pin the official SwiftPM artifact and verify it before extracting executable code.
set -euo pipefail
cd "$(dirname "$0")/.."
VERSION=2.9.3
SHA256=3a5d7fd698acc39c122e75764ed3614b472b284cc483f32ae7006d86c513370c
BASE="$PWD/build/sparkle"
ARCHIVE="$BASE/Sparkle-$VERSION.zip"
mkdir -p "$BASE"
if [ ! -f "$ARCHIVE" ]; then
  curl --fail --location --retry 3 \
    "https://github.com/sparkle-project/Sparkle/releases/download/$VERSION/Sparkle-for-Swift-Package-Manager.zip" \
    --output "$ARCHIVE.download" >&2
  mv "$ARCHIVE.download" "$ARCHIVE"
fi
printf '%s  %s\n' "$SHA256" "$ARCHIVE" | shasum -a 256 --check >&2
# Re-extract from the verified archive rather than trusting a stale SDK directory.
rm -rf "$BASE/sdk"
mkdir -p "$BASE/sdk"
ditto -x -k "$ARCHIVE" "$BASE/sdk"
printf '%s\n' "$BASE/sdk"
