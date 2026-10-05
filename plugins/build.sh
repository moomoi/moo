#!/usr/bin/env bash
# Build every plugin into plugins/dist/, by the tier its moo.json declares:
#   A: bytecode chunk (<id>.tishc), run by the shell in a capability-free VM
#   B: native module (<id>.lib), loaded in-process through tish:ffi
set -euo pipefail
cd "$(dirname "$0")"
TISH="${TISH:-/Users/a_/Projects/tish/tish-nimble/target/release/tish}"
unset CARGO_TARGET_DIR
export TISH_NATIVE_TARGET_DIR="${TISH_NATIVE_TARGET_DIR:-$(cd .. && pwd)/target/tish-native}"
mkdir -p dist
for manifest in */moo.json; do
  dir="${manifest%/moo.json}"
  id="$(rg -o '"id": *"[^"]+"' "$manifest" | sed -E 's/.*"([^"]+)"$/\1/')"
  tier="$(rg -o '"tier": *"[AB]"' "$manifest" | sed -E 's/.*"([AB])"$/\1/')"
  entry="$dir/$(rg -o '"entry": *"[^"]+"' "$manifest" | sed -E 's/.*"([^"]+)"$/\1/')"
  case "$tier" in
    A) "$TISH" build "$entry" --target bytecode -o "dist/$id.tishc" ;;
    B) "$TISH" build "$entry" --target native --crate-type cdylib -o "dist/$id.lib" ;;
    *) echo "skip $dir: unknown tier '$tier'"; continue ;;
  esac
  ls -la dist/"$id".*
done
