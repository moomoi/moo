#!/usr/bin/env bash
# Package Moo for Windows (x64):
#   dist/windows/Moo/            moo.exe, moo.com, plugins\ (bytecode plugins), AppIcon.ico
#   dist/release/Moo-windows-x64.zip
# Runs on Windows (Git Bash) or cross from macOS/Linux with cargo-xwin and LLVM
# (`cargo install cargo-xwin`, `brew install llvm lld`). moo.exe is a GUI program: no console
# window. moo.com is the same binary marked as a console program, for the `moo` command: a shell
# runs `moo` as moo.com (.COM comes first in PATHEXT), waits for it and shows its output.
# Native (.lib) plugins aren't built for Windows yet, so only bytecode plugins ship.
#   TISH          the compiler (default: node_modules/.bin/tish)
#   MOO_VERSION   baked into the binary (default "dev")
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TARGET=x86_64-pc-windows-msvc
TISH="${TISH:-$ROOT/node_modules/.bin/tish}"
OUT="$ROOT/dist/windows/Moo"
export MOO_VERSION="${MOO_VERSION:-dev}"

case "$(uname -s)" in
  MINGW* | MSYS* | CYGWIN*) ;;
  *)
    export PATH="/opt/homebrew/opt/llvm/bin:$PATH"
    # cargo-xwin's C compiler, linker and Windows SDK, for the compiler's own `cargo build`.
    eval "$(cargo xwin env --target "$TARGET")"
    ;;
esac
export TISH_NATIVE_CARGO_TARGET="$TARGET"
export TISH_NATIVE_TARGET_DIR="${TISH_NATIVE_TARGET_DIR:-$ROOT/target/tish-native-win}"

rm -rf "$OUT" && mkdir -p "$OUT/plugins" "$ROOT/dist/release"
(cd "$ROOT/app" && "$TISH" build src/main.tish --target native --native-backend rust --platform windows -o "$OUT/moo.exe")

# moo.exe is marked a GUI program (no console window) and moo.com, a copy, a console program: the
# PE optional header's Subsystem field (e_lfanew + 24 + 68) is 2 or 3. Setting it after the build
# keeps the entry point and doesn't depend on how the compiler passes linker flags.
node -e '
const fs = require("fs");
const exe = process.argv[1], com = process.argv[2];
const b = fs.readFileSync(exe);
if (b.toString("latin1", b.readUInt32LE(0x3c), b.readUInt32LE(0x3c) + 4) !== "PE\0\0") throw new Error("moo.exe is not a PE file");
const at = b.readUInt32LE(0x3c) + 24 + 68;
b.writeUInt16LE(2, at);
fs.writeFileSync(exe, b);
b.writeUInt16LE(3, at);
fs.writeFileSync(com, b);
' "$OUT/moo.exe" "$OUT/moo.com"

bash "$ROOT/scripts/fetch-plugins.sh" >/dev/null
cp "$ROOT"/dist/plugins/*.tishc "$OUT/plugins/"
cp "$ROOT/packaging/AppIcon.ico" "$OUT/"

ZIP="$ROOT/dist/release/Moo-windows-x64.zip"
rm -f "$ZIP"
(cd "$ROOT/dist/windows" && zip -qr "$ZIP" Moo)
unzip -l "$ZIP"
