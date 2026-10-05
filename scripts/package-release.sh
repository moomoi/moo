#!/usr/bin/env bash
# Turn the universal build into release assets in dist/release/:
#   Moo-macos-universal.dmg   signed, notarized, stapled (the app inside is stapled too)
#   Moo-macos-universal.zip   the stapled app, for Homebrew and scripted installs
#   SHA256SUMS
# Runs after scripts/build-universal.sh. Env: VERSION, BUILD, SIGN_IDENTITY, APPLE_ID,
# APPLE_PASSWORD, APPLE_TEAM_ID (each step warns and continues unsigned without them).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ROOT/dist/release"
APP="$ROOT/dist/Moo.app"
NAME="Moo-macos-universal"

SKIP_BUILD=1 MOO_BIN="$ROOT/dist/universal/moo" MOO_PLUGIN_DIST="$ROOT/dist/universal/plugins" \
  bash "$ROOT/scripts/bundle-macos.sh"
bash "$ROOT/scripts/notarize.sh" "$APP"

rm -rf "$OUT"
mkdir -p "$OUT"
ditto -c -k --sequesterRsrc --keepParent "$APP" "$OUT/$NAME.zip"
bash "$ROOT/scripts/make-dmg.sh" "$APP" "$OUT/$NAME.dmg"
bash "$ROOT/scripts/notarize.sh" "$OUT/$NAME.dmg"
bash "$ROOT/scripts/verify-release.sh" "$OUT/$NAME.dmg"

(cd "$OUT" && shasum -a 256 "$NAME.dmg" "$NAME.zip" > SHA256SUMS && cat SHA256SUMS)
