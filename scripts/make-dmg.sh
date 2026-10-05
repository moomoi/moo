#!/usr/bin/env bash
# Pack Moo.app into a compressed DMG with an Applications link to drag it onto.
#
#   bash scripts/make-dmg.sh dist/Moo.app dist/release/Moo-macos-universal.dmg
#
# Signs the DMG when SIGN_IDENTITY is set (notarization wants the container signed too).
set -euo pipefail
APP="${1:?usage: make-dmg.sh <Moo.app> <out.dmg>}"
DMG="${2:?usage: make-dmg.sh <Moo.app> <out.dmg>}"
[ -d "$APP" ] || { echo "error: no such app: $APP" >&2; exit 1; }

STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
ditto "$APP" "$STAGE/$(basename "$APP")"
ln -s /Applications "$STAGE/Applications"

mkdir -p "$(dirname "$DMG")"
rm -f "$DMG"
hdiutil create -volname Moo -srcfolder "$STAGE" -fs HFS+ -format UDZO -ov "$DMG"
if [ -n "${SIGN_IDENTITY:-}" ]; then
  codesign --force --timestamp -s "$SIGN_IDENTITY" "$DMG"
fi
ls -la "$DMG"
