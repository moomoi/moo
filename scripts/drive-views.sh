#!/usr/bin/env bash
# Exercise Lattish plugin views (plugins/hello-list, @nimble/ui) in a running Nimble (NIMBLE_DEBUG=1).
#   NIMBLE_DEBUG=1 NIMBLE_HOTKEY=ctrl+alt+space app/dist/nimble 2>/tmp/nimble.log &
#   HOTKEY_MODS="control down, option down" bash scripts/drive-views.sh
set -euo pipefail
OUT="${1:-/tmp/nimble-views}"
mkdir -p "$OUT"
LOG="${NIMBLE_LOG:-/tmp/nimble.log}"
SAVED_CLIP="$(pbpaste || true)"
trap 'printf %s "$SAVED_CLIP" | pbcopy' EXIT

has_keys() { [ "$(rg -o 'panel key=(true|false)' "$LOG" | tail -1)" = "panel key=true" ]; }
shot() { screencapture -x "$OUT/$1.png"; sips -Z 1600 "$OUT/$1.png" --out "$OUT/$1.png" >/dev/null; echo "shot $1"; }
guard() { has_keys || { echo "abort: nimble panel does not have keyboard focus"; exit 1; }; }
type() { guard; osascript -e "tell application \"System Events\" to keystroke \"$1\""; sleep "${2:-0.7}"; }
key() { guard; osascript -e "tell application \"System Events\" to key code $1"; sleep "${2:-0.5}"; }
clear() { guard; osascript -e 'tell application "System Events" to keystroke "a" using {command down}'; key 51; }
clip() { echo "clipboard: '$(pbpaste)'"; }

if ! has_keys; then
  osascript -e "tell application \"System Events\" to key code 49 using {${HOTKEY_MODS:-option down}}"
  sleep 0.8
fi

type "hello list"
key 36
shot 1-hello-list
type "hola"
key 36
shot 2-starred-hola
clear
shot 3-all-starred-first
for _ in 1 2 3 4 5 6 7 8 9; do key 125 0.15; done
shot 4-scrolled
key 53
type "change case"
key 36
shot 5-change-case-empty
type "hello big world"
shot 6-change-case
key 125
key 125
key 125
key 36
clip
shot 7-copied
key 53
key 53
