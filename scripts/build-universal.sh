#!/usr/bin/env bash
# Build Moo for Apple silicon and Intel and lipo each native artifact into one universal file:
#   dist/universal/moo           the app binary
#   dist/universal/plugins/      the moomoi/plugins release (fetch-plugins.sh): .tishc + universal .lib
# Each slice is built with an explicit target triple (TISH_NATIVE_CARGO_TARGET, in tish since
# v3.15), so neither targets the build machine's CPU.
#   MOO_VERSION   baked into the binary (`moo --version`); default "dev"
#   TISH          the compiler (default: node_modules/.bin/tish, from npm)
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# The compiler comes from npm (package.json); app/build.sh uses $TISH.
[ -n "${TISH:-}" ] || [ -x "$ROOT/node_modules/.bin/tish" ] || (cd "$ROOT" && npm ci --no-audit --no-fund)
export TISH="${TISH:-$ROOT/node_modules/.bin/tish}"
OUT="$ROOT/dist/universal"
SLICES="$ROOT/dist/slices"
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-14.0}"
export MOO_VERSION="${MOO_VERSION:-dev}"
# Slices are signed when bundled; a signature made before lipo would not survive it.
export MOO_SIGN_IDENTITY="-"

mkdir -p "$OUT/plugins" "$SLICES"
for arch in arm64 x86_64; do
  case "$arch" in
    arm64) triple=aarch64-apple-darwin ;;
    x86_64) triple=x86_64-apple-darwin ;;
  esac
  echo "== $arch ($triple)"
  mkdir -p "$SLICES/$arch"
  TISH_NATIVE_CARGO_TARGET="$triple" MOO_OUT="$SLICES/$arch/moo" bash "$ROOT/app/build.sh"
done

lipo -create "$SLICES/arm64/moo" "$SLICES/x86_64/moo" -output "$OUT/moo"
# Plugins come built from their own release, native ones already universal.
bash "$ROOT/scripts/fetch-plugins.sh"
cp "$ROOT"/dist/plugins/*.tishc "$ROOT"/dist/plugins/*.lib "$OUT/plugins/" 2>/dev/null || true

for f in "$OUT/moo" "$OUT"/plugins/*.lib; do
  [ -e "$f" ] || continue
  archs="$(lipo -archs "$f")"
  echo "$(basename "$f"): $archs"
  case "$archs" in
    *x86_64*arm64* | *arm64*x86_64*) ;;
    *) echo "error: $f is not universal" >&2; exit 1 ;;
  esac
done
