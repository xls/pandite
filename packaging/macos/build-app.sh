#!/usr/bin/env bash
# Build the universal pandite.app, sign it and pack it as .dmg and .zip.
#
# Usage: packaging/macos/build-app.sh <version> <out_dir> [<cargo_target_dir>]
#
# Needs both release binaries:
#   <cargo_target_dir>/aarch64-apple-darwin/release/pandite
#   <cargo_target_dir>/x86_64-apple-darwin/release/pandite
# Writes <out_dir>/pandite-<version>-universal-apple-darwin.dmg and .zip.
# PANDITE_BUNDLE_ID overrides the placeholder bundle identifier.
#
# Signature:
#   PANDITE_MACOS_SIGN_IDENTITY  a "Developer ID Application: ..." identity in
#                                the keychain. Without it the app gets an ad
#                                hoc signature, which Gatekeeper does not trust.
#   PANDITE_MACOS_KEYCHAIN       the keychain that holds the identity (optional)
# Notarization (only with a Developer ID signature), both together:
#   PANDITE_APPLE_ID, PANDITE_APPLE_APP_PASSWORD
#                                (an app-specific password of the Apple ID)
#   PANDITE_APPLE_TEAM_ID        optional: read from the identity name
#                                when it is not set
# With them the app and the disk image go to the Apple notary service and
# get their tickets stapled, so they open without a warning, also offline.
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

IDENTITY=${PANDITE_MACOS_SIGN_IDENTITY:-}
KEYCHAIN_ARGS=()
if [ -n "${PANDITE_MACOS_KEYCHAIN:-}" ]; then
  KEYCHAIN_ARGS=(--keychain "$PANDITE_MACOS_KEYCHAIN")
fi
# The team identifier stands in parentheses at the end of the identity name.
TEAM_ID=${PANDITE_APPLE_TEAM_ID:-}
if [ -z "$TEAM_ID" ]; then
  TEAM_ID=$(printf '%s' "$IDENTITY" | sed -n 's/.*(\([A-Z0-9]\{10\}\))$/\1/p')
fi
NOTARIZE=0
if [ -n "$IDENTITY" ] && [ -n "${PANDITE_APPLE_ID:-}" ] && [ -n "$TEAM_ID" ] && [ -n "${PANDITE_APPLE_APP_PASSWORD:-}" ]; then
  NOTARIZE=1
fi

# Send a file to the Apple notary service and wait for the answer. On a
# refusal, print the log of the service and stop.
notarize() {
  local file=$1 result id
  result=$(xcrun notarytool submit "$file" --wait --output-format json \
    --apple-id "$PANDITE_APPLE_ID" --team-id "$TEAM_ID" --password "$PANDITE_APPLE_APP_PASSWORD")
  echo "$result"
  if ! echo "$result" | grep -q '"status" *: *"Accepted"'; then
    id=$(echo "$result" | sed -n 's/.*"id" *: *"\([^"]*\)".*/\1/p' | head -n 1)
    if [ -n "$id" ]; then
      xcrun notarytool log "$id" --apple-id "$PANDITE_APPLE_ID" --team-id "$TEAM_ID" \
        --password "$PANDITE_APPLE_APP_PASSWORD" >&2 || true
    fi
    echo "the Apple notary service did not accept $file" >&2
    exit 1
  fi
}

if [ -n "$IDENTITY" ]; then
  # Developer ID signature with the hardened runtime and a secure time
  # stamp: the notary service requires both.
  codesign --force --options runtime --timestamp --sign "$IDENTITY" ${KEYCHAIN_ARGS[@]+"${KEYCHAIN_ARGS[@]}"} "$APP"
else
  # Ad hoc signature: Apple Silicon refuses unsigned code, and the bundle
  # signature seals Info.plist and the resources.
  codesign --force --sign - --timestamp=none "$APP"
fi
codesign --verify --strict --verbose=2 "$APP"

rm -f "$OUT/$NAME.zip" "$OUT/$NAME.dmg"
if [ "$NOTARIZE" = 1 ]; then
  # The service takes an archive. The ticket then goes into the app, and
  # the zip and the disk image below carry the app with its ticket.
  ditto -c -k --sequesterRsrc --keepParent "$APP" "$WORK/notary.zip"
  notarize "$WORK/notary.zip"
  xcrun stapler staple "$APP"
  xcrun stapler validate "$APP"
fi
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
if [ -n "$IDENTITY" ]; then
  codesign --force --timestamp --sign "$IDENTITY" ${KEYCHAIN_ARGS[@]+"${KEYCHAIN_ARGS[@]}"} "$OUT/$NAME.dmg"
fi
if [ "$NOTARIZE" = 1 ]; then
  notarize "$OUT/$NAME.dmg"
  xcrun stapler staple "$OUT/$NAME.dmg"
  xcrun stapler validate "$OUT/$NAME.dmg"
fi

echo "wrote $OUT/$NAME.dmg"
echo "wrote $OUT/$NAME.zip"
