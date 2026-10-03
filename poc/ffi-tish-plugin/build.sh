#!/usr/bin/env bash
# Stage 0: compile plugin.tish to a Rust lib, wrap it as a decoupled tish_ffi cdylib, stage it
# for poc/host-vm/demo.tish. Run the demo with: $TISH run poc/host-vm/demo.tish
set -euo pipefail
cd "$(dirname "$0")"
TISH="${TISH:-/Users/a_/Projects/tish/tish/target/release/tish}"
unset CARGO_TARGET_DIR

"$TISH" build plugin/src/plugin.tish --target rust-lib --feature timers -o plugin/gen
( cd wrapper && cargo build --release )

mkdir -p ../host-vm/plugins
for cand in libhello_plugin.dylib libhello_plugin.so hello_plugin.dll; do
  if [[ -f "wrapper/target/release/$cand" ]]; then
    cp "wrapper/target/release/$cand" ../host-vm/plugins/hello.lib
    echo "staged $cand -> poc/host-vm/plugins/hello.lib"
    exit 0
  fi
done
echo "error: built cdylib not found" >&2
exit 1
