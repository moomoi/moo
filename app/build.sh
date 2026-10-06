#!/usr/bin/env bash
# Build the Moo shell as a native macOS binary (no Node, no webview, no JS runtime).
# Everything comes from package managers: the compiler from npm (../package.json), tish-macos from
# npm (package.json), and the tish crates both of them and moo-macos use from crates.io.
set -euo pipefail
cd "$(dirname "$0")"

[ -x ../node_modules/.bin/tish ] || (cd .. && npm ci --no-audit --no-fund)
npm ci --no-audit --no-fund
TISH="${TISH:-$(cd .. && pwd)/node_modules/.bin/tish}"
mkdir -p dist

unset CARGO_TARGET_DIR
# Matches LSMinimumSystemVersion; also makes the Swift runtime link from /usr/lib/swift (see .cargo/config.toml).
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-14.0}"
export TISH_NATIVE_TARGET_DIR="${TISH_NATIVE_TARGET_DIR:-$(cd .. && pwd)/target/tish-native}"
OUT="${MOO_OUT:-dist/moo}"
"$TISH" build src/main.tish --target native --native-backend rust -o "$OUT"

# An ad hoc signature is a new identity on every build, so the Keychain asks for the login
# password before Moo can read its own saved keys, and Accessibility forgets it. Signing with a
# certificate under a fixed identifier keeps one identity across rebuilds. MOO_SIGN_IDENTITY picks
# the certificate; "-" keeps ad hoc. Default: the first Apple Development certificate, if any.
IDENTITY="${MOO_SIGN_IDENTITY:-$(security find-identity -v -p codesigning 2>/dev/null | sed -n 's/.*"\(Apple Development: [^"]*\)".*/\1/p' | head -1)}"
if [ -n "$IDENTITY" ] && [ "$IDENTITY" != "-" ]; then
  codesign --force --sign "$IDENTITY" --identifier dev.moo.launcher "$OUT"
fi
ls -la "$OUT"
