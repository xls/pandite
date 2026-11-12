#!/usr/bin/env bash
# Pack the Linux release: binary, desktop entry, icons, install script.
#
# Usage: packaging/linux/build-tarball.sh <version> <out_dir> [<binary>]
#
# <binary> defaults to target/x86_64-unknown-linux-gnu/release/pandite.
# Writes <out_dir>/pandite-<version>-x86_64-unknown-linux-gnu.tar.gz with
# fixed owners, order and times, so equal inputs give equal archives.
set -euo pipefail

VERSION=${1:?usage: build-tarball.sh <version> <out_dir> [<binary>]}
OUT=${2:?usage: build-tarball.sh <version> <out_dir> [<binary>]}
TARGET=x86_64-unknown-linux-gnu
BIN=${3:-target/$TARGET/release/pandite}

HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
NAME="pandite-$VERSION-$TARGET"

[ -f "$BIN" ] || { echo "binary not found: $BIN" >&2; exit 1; }
mkdir -p "$OUT"
OUT=$(cd "$OUT" && pwd)

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
D="$WORK/$NAME"
mkdir -p "$D/share/applications" "$D/share/icons"

install -m 755 "$BIN" "$D/pandite"
install -m 755 "$HERE/install.sh" "$D/install.sh"
install -m 644 "$HERE/pandite.desktop" "$D/share/applications/pandite.desktop"
cp -R "$HERE/icons/hicolor" "$D/share/icons/hicolor"
install -m 644 "$ROOT/LICENSE" "$D/LICENSE"
install -m 644 "$ROOT/README.md" "$D/README.md"
install -m 644 "$ROOT/docs/THIRD-PARTY-NOTICES.md" "$D/THIRD-PARTY-NOTICES.md"
find "$D/share" -type d -exec chmod 755 {} +
find "$D/share" -type f -exec chmod 644 {} +

if command -v desktop-file-validate >/dev/null 2>&1; then
  desktop-file-validate "$D/share/applications/pandite.desktop"
fi

tar --sort=name --owner=0 --group=0 --numeric-owner --mtime="@${SOURCE_DATE_EPOCH:-0}" \
  -C "$WORK" -cf - "$NAME" | gzip -n -9 > "$OUT/$NAME.tar.gz"
echo "wrote $OUT/$NAME.tar.gz"
