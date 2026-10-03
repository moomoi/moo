#!/usr/bin/env bash
# Build the Nimble shell as a native macOS binary (no Node, no webview, no JS runtime).
# Compiler, vendored tish-macos and nimble-macos all use the tish-nimble crates (one tishlang_core);
# see scripts/vendor-tish-apple.sh.
set -euo pipefail
cd "$(dirname "$0")"

TISH="${TISH:-/Users/a_/Projects/tish/tish-nimble/target/release/tish}"
[ -d ../vendor/tish-apple/crates/tish-macos ] || bash ../scripts/vendor-tish-apple.sh
mkdir -p node_modules dist
ln -sfn ../../vendor/tish-apple/crates/tish-macos node_modules/tish-macos
rm -f node_modules/nimble-macos
ln -sfn ../../packages/nimble-macos node_modules/tish-nimble

unset CARGO_TARGET_DIR
export TISH_NATIVE_TARGET_DIR="${TISH_NATIVE_TARGET_DIR:-$(cd .. && pwd)/target/tish-native}"
"$TISH" build src/main.tish --target native --native-backend rust -o dist/nimble
ls -la dist/nimble
