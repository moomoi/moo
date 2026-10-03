#!/usr/bin/env bash
# Show Nimble, type file queries, and screenshot the merged app + Spotlight results.
# Same safety guard as drive-demo.sh: keystrokes are only sent while Nimble's panel has keys.
set -euo pipefail
OUT="${1:-/tmp/nimble-files}"
mkdir -p "$OUT"
LOG="${NIMBLE_LOG:-/tmp/nimble.log}"

has_keys() { [ "$(rg -o 'panel key=(true|false)' "$LOG" | tail -1)" = "panel key=true" ]; }
shot() { screencapture -x "$OUT/$1.png"; sips -Z 1600 "$OUT/$1.png" --out "$OUT/$1.png" >/dev/null; echo "shot $1"; }
guard() { has_keys || { echo "abort: nimble panel does not have keyboard focus"; exit 1; }; }
type() { guard; osascript -e "tell application \"System Events\" to keystroke \"$1\""; sleep "${2:-0.8}"; }
clear_query() { guard; osascript -e 'tell application "System Events" to keystroke "a" using {command down}'; osascript -e 'tell application "System Events" to key code 51'; sleep 0.3; }

if ! has_keys; then
  osascript -e "tell application \"System Events\" to key code 49 using {${HOTKEY_MODS:-option down}}"
  sleep 0.8
fi

i=1
for q in ${QUERIES:-"tish" "plan.md" "readme"}; do
  clear_query
  type "$q" 1.2
  shot "$i-$(printf %s "$q" | tr -c 'a-zA-Z0-9' '_')"
  i=$((i + 1))
done
clear_query
osascript -e 'tell application "System Events" to key code 53'
rg "files |apps reindexed" "$LOG" | tail -10
