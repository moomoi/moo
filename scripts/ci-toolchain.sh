#!/usr/bin/env bash
# Build the pinned Tish toolchain for CI, which has no sibling checkouts:
#   1. clone tishlang/tish at TISH_REF into TISH_DIR, apply TISH_PATCHES, build the compiler
#   2. clone tishlang/tish-apple at APPLE_REF and run scripts/vendor-tish-apple.sh against TISH_DIR
# TISH_DIR defaults to ../../tish/tish-nimble from the repo, the path packages/moo-macos/Cargo.toml
# names: Cargo treats the same crates at two paths as two crates, so the compiler, the vendored
# tish-macos and moo-macos must all use this one copy.
# Prints TISH=<compiler> and, under GitHub Actions, adds it to $GITHUB_ENV.
set -euo pipefail
# Physical paths (pwd -P): Cargo resolves moo-macos's relative path through symlinks, so a logical
# path (macOS /tmp -> /private/tmp) would name the crates a second way.
ROOT="$(cd "$(dirname "$0")/.." && pwd -P)"
# shellcheck source=../toolchain.env
source "$ROOT/toolchain.env"
TISH_DIR="${TISH_DIR:-$(cd "$ROOT/../.." && pwd -P)/tish/tish-nimble}"
APPLE_DIR="${APPLE_DIR:-$ROOT/.toolchain/tish-apple}"
MARK=".moo-ci-toolchain"

checkout() { # <repo url> <dir> <ref>
  # A restored CI cache leaves only target/ behind; anything else is someone's checkout.
  if [ -e "$2" ] && [ ! -e "$2/$MARK" ] && [ -n "$(find "$2" -mindepth 1 -maxdepth 1 ! -name target -print -quit)" ]; then
    echo "error: $2 exists and was not made by this script; it is not reset. Set TISH_DIR / APPLE_DIR elsewhere." >&2
    exit 1
  fi
  if [ ! -d "$2/.git" ]; then
    mkdir -p "$2"
    git init -q "$2"
    git -C "$2" remote add origin "$1"
    touch "$2/$MARK"
  fi
  git -C "$2" fetch -q --depth 1 origin "$3"
  git -C "$2" reset -q --hard FETCH_HEAD
  git -C "$2" clean -qfd -e target -e "$MARK"
}

checkout https://github.com/tishlang/tish.git "$TISH_DIR" "$TISH_REF"
for p in $TISH_PATCHES; do
  git -C "$TISH_DIR" apply --whitespace=nowarn "$ROOT/patches/$p"
  echo "applied $p"
done
(cd "$TISH_DIR" && cargo build --release -p tishlang --features full)

checkout https://github.com/tishlang/tish-apple.git "$APPLE_DIR" "$APPLE_REF"
APPLE_SRC="$APPLE_DIR" TISH_SRC="$TISH_DIR" bash "$ROOT/scripts/vendor-tish-apple.sh"

TISH="$TISH_DIR/target/release/tish"
"$TISH" --version
echo "TISH=$TISH"
if [ -n "${GITHUB_ENV:-}" ]; then
  echo "TISH=$TISH" >> "$GITHUB_ENV"
  echo "TISH_DIR=$TISH_DIR" >> "$GITHUB_ENV"
fi
