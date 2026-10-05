#!/usr/bin/env bash
# Build the moo.moi server (landing page + sign-in relay) as a native binary. No Node, no JS.
# Run it with PORT=<port> ./dist/moo-web (default 8080); see docs/web.md.
set -euo pipefail
cd "$(dirname "$0")"

TISH="${TISH:-/Users/a_/Projects/tish/tish-nimble/target/release/tish}"
mkdir -p dist
unset CARGO_TARGET_DIR
export TISH_NATIVE_TARGET_DIR="${TISH_NATIVE_TARGET_DIR:-$(cd .. && pwd)/target/tish-native-web}"
"$TISH" build src/main.tish --target native --native-backend rust --feature http --feature process -o dist/moo-web
ls -la dist/moo-web
