#!/bin/sh
# Install pandite from this folder.
#
# Usage:
#   ./install.sh                         install for the current user (~/.local)
#   sudo PREFIX=/usr/local ./install.sh  install for every user
#   ./install.sh --uninstall             remove what install.sh installed
set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
PREFIX=${PREFIX:-"$HOME/.local"}
APPS="$PREFIX/share/applications"
ICONS="$PREFIX/share/icons/hicolor"

refresh() {
  if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$APPS" 2>/dev/null || true
  fi
  if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -q -t "$ICONS" 2>/dev/null || true
  fi
}

if [ "${1:-}" = "--uninstall" ]; then
  rm -f "$PREFIX/bin/pandite" "$APPS/pandite.desktop"
  for dir in "$HERE"/share/icons/hicolor/*/; do
    size=$(basename "$dir")
    rm -f "$ICONS/$size/apps/pandite.png"
  done
  refresh
  echo "pandite removed from $PREFIX"
  exit 0
fi

install -D -m 755 "$HERE/pandite" "$PREFIX/bin/pandite"
mkdir -p "$APPS"
# Use the absolute path, so the menu entry works when PREFIX/bin is not on PATH.
sed -e "s|^Exec=pandite|Exec=$PREFIX/bin/pandite|" -e "s|^TryExec=pandite|TryExec=$PREFIX/bin/pandite|" \
  "$HERE/share/applications/pandite.desktop" > "$APPS/pandite.desktop"
chmod 644 "$APPS/pandite.desktop"
for dir in "$HERE"/share/icons/hicolor/*/; do
  size=$(basename "$dir")
  install -D -m 644 "$dir/apps/pandite.png" "$ICONS/$size/apps/pandite.png"
done
refresh
echo "pandite installed in $PREFIX/bin/pandite"
