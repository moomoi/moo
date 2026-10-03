#!/usr/bin/env bash
# Exercise the Tier A (bytecode VM) Unit Converter plugin in a running Nimble (NIMBLE_DEBUG=1).
set -euo pipefail
OUT="${1:-/tmp/nimble-convert}"
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

type "convert"
shot 1-root-convert
key 36
shot 2-convert-empty
type "10 km"
shot 3-10km
key 125
key 36
clip
type " " 0.3
key 51
osascript -e 'tell application "System Events" to keystroke "a" using {command down}'
type "72 f"
shot 4-72f
key 53
type "password"
key 36
clip
shot 5-password
type "a" 0.2
osascript -e 'tell application "System Events" to keystroke "a" using {command down}'
type "converter stats"
key 36
shot 6-stats
key 53
