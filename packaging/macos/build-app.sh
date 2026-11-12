#!/usr/bin/env bash
# Build the universal pandite.app, sign it ad hoc and pack it as .dmg and .zip.
#
# Usage: packaging/macos/build-app.sh <version> <out_dir> [<cargo_target_dir>]
#
# Needs both release binaries:
#   <cargo_target_dir>/aarch64-apple-darwin/release/pandite
#   <cargo_target_dir>/x86_64-apple-darwin/release/pandite
# Writes <out_dir>/pandite-<version>-universal-apple-darwin.dmg and .zip.
# PANDITE_BUNDLE_ID overrides the placeholder bundle identifier.
# Runs on macOS only (lipo, codesign, ditto, hdiutil, plutil).
set -euo pipefail

VERSION=${1:?usage: build-app.sh <version> <out_dir> [<cargo_target_dir>]}
OUT=${2:?usage: build-app.sh <version> <out_dir> [<cargo_target_dir>]}
TARGET_DIR=${3:-target}

HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
ARM="$TARGET_DIR/aarch64-apple-darwin/release/pandite"
X64="$TARGET_DIR/x86_64-apple-darwin/release/pandite"
NAME="pandite-$VERSION-universal-apple-darwin"
# CFBundleVersion takes numbers and dots only: drop a pre-release suffix.
BUNDLE_VERSION=${VERSION%%[-+]*}

for bin in "$ARM" "$X64"; do
  [ -f "$bin" ] || { echo "binary not found: $bin" >&2; exit 1; }
done
mkdir -p "$OUT"
OUT=$(cd "$OUT" && pwd)

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
APP="$WORK/pandite.app"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

lipo -create -output "$APP/Contents/MacOS/pandite" "$ARM" "$X64"
lipo "$APP/Contents/MacOS/pandite" -verify_arch arm64 x86_64
chmod 755 "$APP/Contents/MacOS/pandite"

PLIST="$APP/Contents/Info.plist"
cp "$HERE/Info.plist" "$PLIST"
plutil -replace CFBundleShortVersionString -string "$BUNDLE_VERSION" "$PLIST"
plutil -replace CFBundleVersion -string "$BUNDLE_VERSION" "$PLIST"
if [ -n "${PANDITE_BUNDLE_ID:-}" ]; then
  plutil -replace CFBundleIdentifier -string "$PANDITE_BUNDLE_ID" "$PLIST"
fi
plutil -lint "$PLIST"
printf 'APPL????' > "$APP/Contents/PkgInfo"
cp "$HERE/pandite.icns" "$APP/Contents/Resources/pandite.icns"
cp "$ROOT/LICENSE" "$APP/Contents/Resources/LICENSE.txt"
cp "$ROOT/docs/THIRD-PARTY-NOTICES.md" "$APP/Contents/Resources/THIRD-PARTY-NOTICES.md"

# Ad hoc signature: Apple Silicon refuses unsigned code, and the bundle
# signature seals Info.plist and the resources.
codesign --force --sign - --timestamp=none "$APP"
codesign --verify --strict --verbose=2 "$APP"

rm -f "$OUT/$NAME.zip" "$OUT/$NAME.dmg"
ditto -c -k --sequesterRsrc --keepParent "$APP" "$OUT/$NAME.zip"

DMG_SRC="$WORK/dmg"
mkdir -p "$DMG_SRC"
cp -R "$APP" "$DMG_SRC/"
ln -s /Applications "$DMG_SRC/Applications"
cp "$ROOT/LICENSE" "$DMG_SRC/LICENSE.txt"
# hdiutil fails now and then with "Resource busy" on build machines.
made=0
for attempt in 1 2 3 4 5; do
  if hdiutil create -volname "pandite $VERSION" -srcfolder "$DMG_SRC" -fs HFS+ -format UDZO -ov "$OUT/$NAME.dmg"; then
    made=1
    break
  fi
  echo "hdiutil attempt $attempt failed; retrying" >&2
  sleep $((attempt * 5))
done
[ "$made" = 1 ] || { echo "hdiutil could not create the disk image" >&2; exit 1; }
hdiutil verify "$OUT/$NAME.dmg"

echo "wrote $OUT/$NAME.dmg"
echo "wrote $OUT/$NAME.zip"
