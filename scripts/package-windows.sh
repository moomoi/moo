#!/usr/bin/env bash
# Package Moo for Windows (x64):
#   dist/windows/Moo/            moo.exe, plugins\ (bytecode plugins), AppIcon.ico
#   dist/release/Moo-windows-x64.zip
# Runs on Windows (Git Bash) or cross from macOS/Linux with cargo-xwin and LLVM
# (`cargo install cargo-xwin`, `brew install llvm lld`). moo.exe is a GUI program: no console
# window. Native (.lib) plugins aren't built for Windows yet, so only bytecode plugins ship.
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
# A GUI program: Windows opens no console window for it.
VAR=CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS
export "$VAR=${!VAR:-} -C link-arg=/SUBSYSTEM:WINDOWS -C link-arg=/ENTRY:mainCRTStartup"
export TISH_NATIVE_CARGO_TARGET="$TARGET"
export TISH_NATIVE_TARGET_DIR="${TISH_NATIVE_TARGET_DIR:-$ROOT/target/tish-native-win}"

rm -rf "$OUT" && mkdir -p "$OUT/plugins" "$ROOT/dist/release"
(cd "$ROOT/app" && "$TISH" build src/main.tish --target native --native-backend rust --platform windows -o "$OUT/moo.exe")

bash "$ROOT/scripts/fetch-plugins.sh" >/dev/null
cp "$ROOT"/dist/plugins/*.tishc "$OUT/plugins/"
cp "$ROOT/packaging/AppIcon.ico" "$OUT/"

ZIP="$ROOT/dist/release/Moo-windows-x64.zip"
rm -f "$ZIP"
(cd "$ROOT/dist/windows" && zip -qr "$ZIP" Moo)
unzip -l "$ZIP"
