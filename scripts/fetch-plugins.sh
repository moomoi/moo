#!/usr/bin/env bash
# Put the plugins Moo ships into dist/plugins: the moomoi/plugins release named in plugins.version
# (moo-plugins.tar.gz, checked against its SHA256SUMS). Does nothing when that version is already
# there. Plugins are built and released in moomoi/plugins, never here.
#   MOO_PLUGINS_TARBALL   use this local moo-plugins.tar.gz instead (testing a plugins build)
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VERSION="$(tr -d '[:space:]' < "$ROOT/plugins.version")"
DEST="$ROOT/dist/plugins"
STAMP="$DEST/.version"
WANT="${MOO_PLUGINS_TARBALL:-v$VERSION}"

if [ -f "$STAMP" ] && [ "$(cat "$STAMP")" = "$WANT" ]; then
  exit 0
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
if [ -n "${MOO_PLUGINS_TARBALL:-}" ]; then
  cp "$MOO_PLUGINS_TARBALL" "$WORK/moo-plugins.tar.gz"
else
  BASE="https://github.com/moomoi/plugins/releases/download/v$VERSION"
  echo "Downloading plugins v$VERSION..."
  curl -fsSL "$BASE/moo-plugins.tar.gz" -o "$WORK/moo-plugins.tar.gz"
  curl -fsSL "$BASE/SHA256SUMS" -o "$WORK/SHA256SUMS"
  (cd "$WORK" && shasum -a 256 -c SHA256SUMS >/dev/null) || { echo "error: moo-plugins.tar.gz does not match SHA256SUMS" >&2; exit 1; }
fi

rm -rf "$DEST"
mkdir -p "$DEST"
tar -xzf "$WORK/moo-plugins.tar.gz" -C "$DEST"
echo "$WANT" > "$STAMP"
ls "$DEST"
