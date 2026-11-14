#!/usr/bin/env bash
# Build the RPM package from the Linux release tarball.
#
# Usage: packaging/linux/build-rpm.sh <version> <out_dir> <tarball>
#
# <tarball> is pandite-<version>-x86_64-linux.tar.gz from
# build-tarball.sh. Writes <out_dir>/pandite-<version>-1.x86_64.rpm with the
# same program, desktop entry and icons under /usr (packaging/linux/pandite.spec).
# Needs rpmbuild (Debian, Ubuntu: apt install rpm; Fedora: dnf install rpm-build).
set -euo pipefail

usage='usage: build-rpm.sh <version> <out_dir> <tarball>'
VERSION=${1:?$usage}
OUT=${2:?$usage}
TARBALL=${3:?$usage}
TARGET=x86_64-linux
NAME="pandite-$VERSION-$TARGET"

HERE=$(cd "$(dirname "$0")" && pwd)

for tool in rpmbuild rpm; do
  command -v "$tool" >/dev/null 2>&1 || { echo "$tool not found: install rpm (Debian, Ubuntu) or rpm-build (Fedora)" >&2; exit 1; }
done
[ -f "$TARBALL" ] || { echo "tarball not found: $TARBALL" >&2; exit 1; }
mkdir -p "$OUT"
OUT=$(cd "$OUT" && pwd)

# RPM versions take no "-" and sort "~" before the release: 0.2.0-rc.1
# becomes 0.2.0~rc.1.
RPMVERSION=$(printf '%s' "$VERSION" | sed -e 's/-/~/' -e 's/-/./g')
export SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-0}"

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
tar -xzf "$TARBALL" -C "$WORK"
SRC="$WORK/$NAME"
[ -f "$SRC/pandite" ] || { echo "$TARBALL has no $NAME/pandite" >&2; exit 1; }

rpmbuild -bb \
  --define "_topdir $WORK/rpmbuild" \
  --define "pandite_version $RPMVERSION" \
  --define "pandite_source $SRC" \
  --define "use_source_date_epoch_as_buildtime 1" \
  --define "clamp_mtime_to_source_date_epoch 1" \
  --define "_buildhost localhost" \
  "$HERE/pandite.spec"

BUILT="$WORK/rpmbuild/RPMS/x86_64/pandite-$RPMVERSION-1.x86_64.rpm"
[ -f "$BUILT" ] || { echo "rpmbuild did not write $BUILT" >&2; exit 1; }
RPM="$OUT/pandite-$VERSION-1.x86_64.rpm"
cp "$BUILT" "$RPM"

rpm -qip "$RPM"
rpm -qlp "$RPM"
echo "Requires:"
REQUIRES=$(rpm -qp --requires "$RPM")
printf '%s\n' "$REQUIRES"
echo "Recommends:"
rpm -qp --recommends "$RPM"
# rpmbuild reads the shared libraries of the program; without them the
# package would install on systems that cannot run it.
if ! printf '%s\n' "$REQUIRES" | grep -q '^libc\.so\.6'; then
  echo "the package does not require libc.so.6: rpmbuild generated no library dependencies" >&2
  exit 1
fi
echo "wrote $RPM"
