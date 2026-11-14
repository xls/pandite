#!/usr/bin/env bash
# Build the Debian package from the Linux release tarball.
#
# Usage: packaging/linux/build-deb.sh <version> <out_dir> <tarball>
#
# <tarball> is pandite-<version>-x86_64-linux.tar.gz from
# build-tarball.sh. Writes <out_dir>/pandite_<version>_amd64.deb with the
# same program, desktop entry and icons under /usr. dpkg-shlibdeps gives
# Depends; packaging/linux/debian/control lists the libraries the program
# loads at run time as Recommends.
# Needs dpkg-deb and dpkg-shlibdeps (Debian, Ubuntu: apt install dpkg-dev).
set -euo pipefail

usage='usage: build-deb.sh <version> <out_dir> <tarball>'
VERSION=${1:?$usage}
OUT=${2:?$usage}
TARBALL=${3:?$usage}
TARGET=x86_64-linux
NAME="pandite-$VERSION-$TARGET"

HERE=$(cd "$(dirname "$0")" && pwd)
TEMPLATE="$HERE/debian/control"

for tool in dpkg-deb dpkg-shlibdeps; do
  command -v "$tool" >/dev/null 2>&1 || { echo "$tool not found: install dpkg-dev" >&2; exit 1; }
done
[ -f "$TARBALL" ] || { echo "tarball not found: $TARBALL" >&2; exit 1; }
mkdir -p "$OUT"
OUT=$(cd "$OUT" && pwd)

# Debian sorts "~" before the release: 0.2.0-rc.1 becomes 0.2.0~rc.1.
DEBVERSION=$(printf '%s' "$VERSION" | sed -e 's/-/~/' -e 's/-/./g')
EPOCH=${SOURCE_DATE_EPOCH:-0}
export SOURCE_DATE_EPOCH="$EPOCH"
HOMEPAGE=$(sed -n 's/^Homepage: //p' "$TEMPLATE")
MAINTAINER=$(sed -n 's/^Maintainer: //p' "$TEMPLATE")
umask 022

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
tar -xzf "$TARBALL" -C "$WORK"
SRC="$WORK/$NAME"
[ -f "$SRC/pandite" ] || { echo "$TARBALL has no $NAME/pandite" >&2; exit 1; }

# dpkg-shlibdeps reads debian/control and expects the tree in debian/<package>.
PKG="$WORK/debian/pandite"
DOC="$PKG/usr/share/doc/pandite"
mkdir -p "$PKG/DEBIAN" "$DOC" "$PKG/usr/share/icons"
install -D -m 755 "$SRC/pandite" "$PKG/usr/bin/pandite"
install -D -m 644 "$SRC/share/applications/pandite.desktop" "$PKG/usr/share/applications/pandite.desktop"
cp -R "$SRC/share/icons/hicolor" "$PKG/usr/share/icons/hicolor"
gzip -9n < "$SRC/README.md" > "$DOC/README.md.gz"
gzip -9n < "$SRC/THIRD-PARTY-NOTICES.md" > "$DOC/THIRD-PARTY-NOTICES.md.gz"

# Machine-readable copyright file with the text of LICENSE.
HOLDER=$(sed -n 's/^Copyright (c) //p' "$SRC/LICENSE" | head -n 1)
[ -n "$HOLDER" ] || { echo "LICENSE has no 'Copyright (c)' line" >&2; exit 1; }
{
  echo 'Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/'
  echo 'Upstream-Name: pandite'
  echo "Source: $HOMEPAGE"
  echo
  echo 'Files: *'
  echo "Copyright: $HOLDER"
  echo 'License: MIT'
  echo 'Comment: The licenses of the Rust crates compiled into the program are in'
  echo ' /usr/share/doc/pandite/THIRD-PARTY-NOTICES.md.gz.'
  echo
  echo 'License: MIT'
  awk '/^Copyright \(c\) / { body = 1; next }
       !body { next }
       $0 == "" { if (started) blank++; next }
       { while (blank > 0) { print " ."; blank-- }; started = 1; print " " $0 }' "$SRC/LICENSE"
} > "$DOC/copyright"

{
  echo "pandite ($DEBVERSION) unstable; urgency=medium"
  echo
  echo "  * Release $VERSION: $HOMEPAGE/releases/tag/v$VERSION"
  echo
  echo " -- $MAINTAINER  $(LC_ALL=C date -u -R -d "@$EPOCH")"
} | gzip -9n > "$DOC/changelog.gz"

find "$PKG" -type d -exec chmod 755 {} +
find "$PKG/usr" -type f -exec chmod 644 {} +
chmod 755 "$PKG/usr/bin/pandite"
install -m 755 "$HERE/debian/postinst" "$PKG/DEBIAN/postinst"
install -m 755 "$HERE/debian/postrm" "$PKG/DEBIAN/postrm"

printf 'Source: pandite\n\nPackage: pandite\nArchitecture: amd64\n' > "$WORK/debian/control"
DEPENDS=$(cd "$WORK" && dpkg-shlibdeps -O debian/pandite/usr/bin/pandite | sed -n 's/^shlibs:Depends=//p')
[ -n "$DEPENDS" ] || { echo "dpkg-shlibdeps found no shared library dependencies" >&2; exit 1; }
SIZE=$(du -sk "$PKG/usr" | cut -f1)

esc() { printf '%s' "$1" | sed -e 's/[\\&|]/\\&/g'; }
sed -e "s|@VERSION@|$(esc "$DEBVERSION")|" \
  -e "s|@INSTALLED_SIZE@|$SIZE|" \
  -e "s|@DEPENDS@|$(esc "$DEPENDS")|" \
  "$TEMPLATE" > "$PKG/DEBIAN/control"
if grep -q '@[A-Z_]*@' "$PKG/DEBIAN/control"; then
  echo "$TEMPLATE has a token this script does not fill" >&2
  exit 1
fi
(cd "$PKG" && find usr -type f -print0 | LC_ALL=C sort -z | xargs -0 md5sum) > "$PKG/DEBIAN/md5sums"
chmod 644 "$PKG/DEBIAN/control" "$PKG/DEBIAN/md5sums"
find "$PKG" -exec touch -h -d "@$EPOCH" {} +

DEB="$OUT/pandite_${VERSION}_amd64.deb"
rm -f "$DEB"
# xz, not the zstd default of Ubuntu: older Debian dpkg cannot read zstd.
dpkg-deb --root-owner-group -Zxz -z9 --build "$PKG" "$DEB"
dpkg-deb --info "$DEB"
dpkg-deb --contents "$DEB"
echo "wrote $DEB"
