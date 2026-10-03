#!/usr/bin/env bash
# Exercise plugin commands in a running Nimble (NIMBLE_DEBUG=1): a no-view command, a list command
# with filtering, copy actions, and esc back to root. Restores the clipboard afterwards.
set -euo pipefail
OUT="${1:-/tmp/nimble-plugins}"
mkdir -p "$OUT"
LOG="${NIMBLE_LOG:-/tmp/nimble.log}"
SAVED_CLIP="$(pbpaste || true)"
trap 'printf %s "$SAVED_CLIP" | pbcopy' EXIT

has_keys() { [ "$(rg -o 'panel key=(true|false)' "$LOG" | tail -1)" = "panel key=true" ]; }
shot() { screencapture -x "$OUT/$1.png"; sips -Z 1600 "$OUT/$1.png" --out "$OUT/$1.png" >/dev/null; echo "shot $1"; }
guard() { has_keys || { echo "abort: nimble panel does not have keyboard focus"; exit 1; }; }
type() { guard; osascript -e "tell application \"System Events\" to keystroke \"$1\""; sleep "${2:-0.7}"; }
key() { guard; osascript -e "tell application \"System Events\" to key code $1"; sleep "${2:-0.5}"; }
clip() { echo "clipboard: '$(pbpaste)'"; }

if ! has_keys; then
  osascript -e "tell application \"System Events\" to key code 49 using {${HOTKEY_MODS:-option down}}"
  sleep 0.8
fi
shot 1-root

type "uuid"
shot 2-uuid-command
key 36
clip
shot 3-uuid-ran

key 51; key 51; key 51; key 51
type "emoji"
key 36
shot 4-emoji-list
type "rock"
shot 5-emoji-filtered
key 36
clip
shot 6-emoji-copied

key 53
shot 7-back-to-root
type "dev links"
key 36
shot 8-links-list
key 53
key 53
echo "after esc esc: panel has keys: $(has_keys && echo yes || echo no)"
