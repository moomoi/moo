#!/usr/bin/env bash
# Put the plugins Moo ships into dist/plugins (macOS) or dist/plugins-windows: the moomoi/plugins
# release named in plugins.version (moo-plugins.tar.gz or moo-plugins-windows.tar.gz, checked
# against its SHA256SUMS). Does nothing when that version is already there. Plugins are built and
# released in moomoi/plugins, never here.
#   bash scripts/fetch-plugins.sh [mac|windows]   (default mac)
#   MOO_PLUGINS_TARBALL   use this local tarball instead (testing a plugins build)
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PLATFORM="${1:-mac}"
case "$PLATFORM" in
  mac) ASSET=moo-plugins.tar.gz; DEST="$ROOT/dist/plugins" ;;
  windows) ASSET=moo-plugins-windows.tar.gz; DEST="$ROOT/dist/plugins-windows" ;;
  *) echo "usage: fetch-plugins.sh [mac|windows]" >&2; exit 2 ;;
esac
VERSION="$(tr -d '[:space:]' < "$ROOT/plugins.version")"
STAMP="$DEST/.version"
WANT="${MOO_PLUGINS_TARBALL:-v$VERSION}"

if [ -f "$STAMP" ] && [ "$(cat "$STAMP")" = "$WANT" ]; then
  exit 0
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
if [ -n "${MOO_PLUGINS_TARBALL:-}" ]; then
  cp "$MOO_PLUGINS_TARBALL" "$WORK/$ASSET"
else
  BASE="https://github.com/moomoi/plugins/releases/download/v$VERSION"
  echo "Downloading $ASSET v$VERSION..."
  curl -fsSL "$BASE/$ASSET" -o "$WORK/$ASSET"
  curl -fsSL "$BASE/SHA256SUMS" -o "$WORK/SHA256SUMS"
  # SHA256SUMS lists every tarball; check the one downloaded.
  (cd "$WORK" && grep " $ASSET\$" SHA256SUMS | shasum -a 256 -c >/dev/null) || { echo "error: $ASSET does not match SHA256SUMS" >&2; exit 1; }
fi

rm -rf "$DEST"
mkdir -p "$DEST"
tar -xzf "$WORK/$ASSET" -C "$DEST"
echo "$WANT" > "$STAMP"
ls "$DEST"
