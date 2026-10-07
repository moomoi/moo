#!/usr/bin/env bash
# Build the moo.moi server as a native binary: render the Lattish site (site/, docs/) to HTML, then
# compile the server (src/main.tish) that serves it and relays sign-ins. No Node, no JS at runtime.
# Run it with PORT=<port> ./dist/moo-web (default 8080); see docs/web.md.
set -euo pipefail
cd "$(dirname "$0")"

[ -n "${TISH:-}" ] || [ -x ../node_modules/.bin/tish ] || (cd .. && npm ci --no-audit --no-fund)
TISH="${TISH:-$(cd .. && pwd)/node_modules/.bin/tish}"
mkdir -p dist
"$TISH" run --feature fs render.tish   # site/ + docs/ -> dist/pages.tish
unset CARGO_TARGET_DIR
export TISH_NATIVE_TARGET_DIR="${TISH_NATIVE_TARGET_DIR:-$(cd .. && pwd)/target/tish-native-web}"
"$TISH" build src/main.tish --target native --native-backend rust --feature http --feature process -o dist/moo-web
ls -la dist/moo-web
