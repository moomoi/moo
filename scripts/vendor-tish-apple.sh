#!/usr/bin/env bash
# Copy tish-macos + tish-apple-common into vendor/ and point their tishlang_* path deps at the
# tish checkout Moo builds with (TISH_SRC, default: the tish-nimble worktree).
#
# Why: Cargo identifies a path crate by its path, so tish-macos linking ~/Projects/tish/tish/crates
# while the compiler emits ~/Projects/tish/tish-nimble/crates gives two incompatible tishlang_core
# crates. Re-run after pulling tish-apple; local changes live in patches/tish-apple-moo.patch.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APPLE_SRC="${APPLE_SRC:-$ROOT/../../tish/tish-apple}"
TISH_SRC="${TISH_SRC:-$ROOT/../../tish/tish-nimble}"
APPLE_SRC="$(cd "$APPLE_SRC" && pwd)"
TISH_CRATES="$(cd "$TISH_SRC/crates" && pwd)"
DEST="$ROOT/vendor/tish-apple/crates"

mkdir -p "$DEST"
for crate in tish-macos tish-apple-common; do
  rsync -a --delete --exclude examples --exclude Cargo.lock --exclude target \
    "$APPLE_SRC/crates/$crate/" "$DEST/$crate/"
  perl -pi -e "s#path = \"\\.\\./\\.\\./\\.\\./tish/crates/#path = \"$TISH_CRATES/#g" "$DEST/$crate/Cargo.toml"
done

if [ -f "$ROOT/patches/tish-apple-moo.patch" ]; then
  (cd "$ROOT/vendor/tish-apple" && patch -p1 --forward --silent < "$ROOT/patches/tish-apple-moo.patch")
fi
rg -n "tishlang_(core|ui|runtime) *=" "$DEST"/*/Cargo.toml
