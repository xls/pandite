#!/usr/bin/env bash
# Build the universal Pandite.app, sign it and pack it as a .dmg.
#
# Usage: packaging/macos/build-app.sh <version> <out_dir> [<cargo_target_dir>]
#
# Needs both release binaries:
#   <cargo_target_dir>/aarch64-apple-darwin/release/pandite
#   <cargo_target_dir>/x86_64-apple-darwin/release/pandite
# Writes <out_dir>/pandite-<version>-universal-macos.dmg.
# PANDITE_BUNDLE_ID overrides the bundle identifier of Info.plist.
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
#
# App Sandbox builds (optional, see docs/mac-app-store.md). Without these
# variables the script makes the Developer ID build above:
#   PANDITE_MACOS_SANDBOX_TEST=1 sign the app of the disk image with the App
#                                Sandbox entitlements (pandite.entitlements)
#                                and write <out_dir>/pandite-<version>-
#                                universal-macos-sandbox-test.dmg, to try the
#                                sandbox before a store submission. Signature
#                                and notarization as above.
#   PANDITE_MACOS_STORE=1        write <out_dir>/pandite-<version>-macos-
#                                appstore.pkg for App Store Connect instead of
#                                the disk image. Needs all three:
#   PANDITE_MACOS_STORE_APP_IDENTITY        an "Apple Distribution: ..." or
#                                "3rd Party Mac Developer Application: ..."
#                                identity in the keychain
#   PANDITE_MACOS_STORE_INSTALLER_IDENTITY  a "3rd Party Mac Developer
#                                Installer: ..." identity in the keychain
#   PANDITE_MACOS_PROVISIONING_PROFILE      the Mac App Store provisioning
#                                profile (.provisionprofile) of the bundle id
#   PANDITE_MACOS_KEYCHAIN applies to these identities too.
# Runs on macOS only (lipo, codesign, ditto, hdiutil, plutil; the store
# package also security, PlistBuddy, productbuild and pkgutil).
set -euo pipefail

VERSION=${1:?usage: build-app.sh <version> <out_dir> [<cargo_target_dir>]}
OUT=${2:?usage: build-app.sh <version> <out_dir> [<cargo_target_dir>]}
TARGET_DIR=${3:-target}

HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
ARM="$TARGET_DIR/aarch64-apple-darwin/release/pandite"
X64="$TARGET_DIR/x86_64-apple-darwin/release/pandite"
NAME="pandite-$VERSION-universal-macos"
# CFBundleVersion takes numbers and dots only: drop a pre-release suffix.
BUNDLE_VERSION=${VERSION%%[-+]*}
ENTITLEMENTS="$HERE/pandite.entitlements"
STORE=0
SANDBOX_TEST=0
if [ "${PANDITE_MACOS_STORE:-}" = 1 ]; then
  STORE=1
fi
if [ "${PANDITE_MACOS_SANDBOX_TEST:-}" = 1 ]; then
  SANDBOX_TEST=1
  NAME="$NAME-sandbox-test"
fi
if [ "$STORE" = 1 ] && [ "$SANDBOX_TEST" = 1 ]; then
  echo "set PANDITE_MACOS_STORE or PANDITE_MACOS_SANDBOX_TEST, not both" >&2
  exit 1
fi

for bin in "$ARM" "$X64"; do
  [ -f "$bin" ] || { echo "binary not found: $bin" >&2; exit 1; }
done
mkdir -p "$OUT"
OUT=$(cd "$OUT" && pwd)

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
APP="$WORK/Pandite.app"
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

if [ "$STORE" = 1 ]; then
  # Mac App Store: the sandbox entitlements plus the application and team
  # identifiers of the provisioning profile, the profile inside the app,
  # and an installer package signed for App Store Connect. No notarization:
  # Apple signs what the store accepts.
  APP_IDENTITY=${PANDITE_MACOS_STORE_APP_IDENTITY:?set PANDITE_MACOS_STORE_APP_IDENTITY to an Apple Distribution identity}
  INSTALLER_IDENTITY=${PANDITE_MACOS_STORE_INSTALLER_IDENTITY:?set PANDITE_MACOS_STORE_INSTALLER_IDENTITY to a 3rd Party Mac Developer Installer identity}
  PROFILE=${PANDITE_MACOS_PROVISIONING_PROFILE:?set PANDITE_MACOS_PROVISIONING_PROFILE to the Mac App Store provisioning profile}
  [ -f "$PROFILE" ] || { echo "provisioning profile not found: $PROFILE" >&2; exit 1; }
  cp "$PROFILE" "$APP/Contents/embedded.provisionprofile"
  security cms -D -i "$PROFILE" > "$WORK/profile.plist"
  PROFILE_APP_ID=$(/usr/libexec/PlistBuddy -c 'Print :Entitlements:com.apple.application-identifier' "$WORK/profile.plist")
  PROFILE_TEAM=$(/usr/libexec/PlistBuddy -c 'Print :TeamIdentifier:0' "$WORK/profile.plist")
  BUNDLE_ID=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$PLIST")
  if [ "$PROFILE_APP_ID" != "$PROFILE_TEAM.$BUNDLE_ID" ]; then
    echo "the provisioning profile is for $PROFILE_APP_ID, the app is $PROFILE_TEAM.$BUNDLE_ID" >&2
    exit 1
  fi
  STORE_ENTITLEMENTS="$WORK/store.entitlements"
  cp "$ENTITLEMENTS" "$STORE_ENTITLEMENTS"
  /usr/libexec/PlistBuddy -c "Add :com.apple.application-identifier string $PROFILE_APP_ID" "$STORE_ENTITLEMENTS"
  /usr/libexec/PlistBuddy -c "Add :com.apple.developer.team-identifier string $PROFILE_TEAM" "$STORE_ENTITLEMENTS"
  plutil -lint "$STORE_ENTITLEMENTS"
  codesign --force --options runtime --timestamp --entitlements "$STORE_ENTITLEMENTS" \
    --sign "$APP_IDENTITY" ${KEYCHAIN_ARGS[@]+"${KEYCHAIN_ARGS[@]}"} "$APP"
  codesign --verify --strict --verbose=2 "$APP"
  PKG="$OUT/pandite-$VERSION-macos-appstore.pkg"
  rm -f "$PKG"
  productbuild --component "$APP" /Applications --sign "$INSTALLER_IDENTITY" \
    ${KEYCHAIN_ARGS[@]+"${KEYCHAIN_ARGS[@]}"} "$PKG"
  pkgutil --check-signature "$PKG"
  echo "wrote $PKG"
  exit 0
fi

if [ "$SANDBOX_TEST" = 1 ]; then
  # The app of the default build with the App Sandbox entitlements. These
  # entitlements need no provisioning profile.
  if [ -n "$IDENTITY" ]; then
    codesign --force --options runtime --timestamp --entitlements "$ENTITLEMENTS" --sign "$IDENTITY" ${KEYCHAIN_ARGS[@]+"${KEYCHAIN_ARGS[@]}"} "$APP"
  else
    codesign --force --sign - --timestamp=none --entitlements "$ENTITLEMENTS" "$APP"
  fi
elif [ -n "$IDENTITY" ]; then
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
  # the disk image below carries the app with its ticket.
  ditto -c -k --sequesterRsrc --keepParent "$APP" "$WORK/notary.zip"
  notarize "$WORK/notary.zip"
  xcrun stapler staple "$APP"
  xcrun stapler validate "$APP"
fi

DMG_SRC="$WORK/dmg"
mkdir -p "$DMG_SRC"
cp -R "$APP" "$DMG_SRC/"
ln -s /Applications "$DMG_SRC/Applications"
cp "$ROOT/LICENSE" "$DMG_SRC/LICENSE.txt"
# hdiutil fails now and then with "Resource busy" on build machines.
made=0
for attempt in 1 2 3 4 5; do
  if hdiutil create -volname "Pandite $VERSION" -srcfolder "$DMG_SRC" -fs HFS+ -format UDZO -ov "$OUT/$NAME.dmg"; then
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
