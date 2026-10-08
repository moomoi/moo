#!/usr/bin/env bash
# Build the Moo shell as a native macOS binary (no Node, no webview, no JS runtime).
# Everything comes from package managers: the compiler from npm (../package.json), tish-macos from
# npm (package.json), and the tish crates both of them and moo-macos use from crates.io.
set -euo pipefail
cd "$(dirname "$0")"

# Install only when the lockfile changed: `npm ci` re-extracts node_modules, and the new file times
# make cargo rebuild tish-macos (a path dependency there) and then the whole app.
fresh() { [ -f "$1/node_modules/.package-lock.json" ] && [ ! "$1/package-lock.json" -nt "$1/node_modules/.package-lock.json" ]; }
fresh .. || (cd .. && npm ci --no-audit --no-fund)
fresh . || npm ci --no-audit --no-fund
TISH="${TISH:-$(cd .. && pwd)/node_modules/.bin/tish}"
mkdir -p dist

unset CARGO_TARGET_DIR
# Matches LSMinimumSystemVersion; also makes the Swift runtime link from /usr/lib/swift (see .cargo/config.toml).
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-14.0}"
export TISH_NATIVE_TARGET_DIR="${TISH_NATIVE_TARGET_DIR:-$(cd .. && pwd)/target/tish-native}"
OUT="${MOO_OUT:-dist/moo}"
# Cargo's progress isn't shown, so say what's happening: a release build (one codegen unit, fat
# LTO) takes minutes from cold; TISH_FAST_NATIVE_BUILD=1 (npm run build) compiles in parallel.
if [ "${TISH_FAST_NATIVE_BUILD:-}" = "1" ]; then
  echo "Compiling the app (fast dev build)..."
else
  echo "Compiling the app (optimized release build; several minutes from cold)..."
fi
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
