#!/usr/bin/env bash
# Notarize and staple Moo.app or a DMG.
#
#   bash scripts/notarize.sh dist/Moo.app
#   bash scripts/notarize.sh dist/release/Moo-macos-universal.dmg
#
# Needs APPLE_ID, APPLE_PASSWORD (app-specific password) and APPLE_TEAM_ID. Without them it warns
# and does nothing, so forks and unsigned builds still package. A Developer ID signature alone
# still gets "Apple could not verify…" on a downloaded app; only a stapled ticket clears it.
set -euo pipefail
TARGET="${1:?usage: notarize.sh <Moo.app|file.dmg>}"
[ -e "$TARGET" ] || { echo "error: no such file: $TARGET" >&2; exit 1; }

if [ -z "${APPLE_ID:-}" ] || [ -z "${APPLE_PASSWORD:-}" ] || [ -z "${APPLE_TEAM_ID:-}" ]; then
  echo "::warning::APPLE_ID / APPLE_PASSWORD / APPLE_TEAM_ID not all set; skipping notarization of $TARGET"
  exit 0
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
SUBMIT="$TARGET"
case "$TARGET" in
  *.app)
    # notarytool takes a zip, dmg or pkg; the ticket it issues is stapled to the app itself.
    SUBMIT="$WORK/$(basename "$TARGET" .app).zip"
    ditto -c -k --sequesterRsrc --keepParent "$TARGET" "$SUBMIT"
    ;;
esac

echo ">> notarizing $TARGET"
xcrun notarytool submit "$SUBMIT" \
  --apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" --team-id "$APPLE_TEAM_ID" \
  --wait --timeout 30m --output-format json > "$WORK/result.json"
cat "$WORK/result.json"; echo
STATUS="$(plutil -extract status raw -o - "$WORK/result.json")"
if [ "$STATUS" != "Accepted" ]; then
  ID="$(plutil -extract id raw -o - "$WORK/result.json")"
  xcrun notarytool log "$ID" --apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" --team-id "$APPLE_TEAM_ID" || true
  echo "error: notarization of $TARGET ended with status $STATUS" >&2
  exit 1
fi
xcrun stapler staple "$TARGET"
xcrun stapler validate "$TARGET"
echo ">> notarized and stapled: $TARGET"
