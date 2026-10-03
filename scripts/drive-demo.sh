#!/usr/bin/env bash
# Drive a running Nimble through hotkey -> type -> edit shortcuts -> select -> launch, with screenshots.
# Every keystroke step first checks that Nimble is frontmost, so nothing is typed into other apps.
# Needs Accessibility permission for the app running this script (System Events keystrokes).
set -euo pipefail
OUT="${1:-/tmp/nimble-demo}"
mkdir -p "$OUT"

SAVED_CLIP="$(pbpaste || true)"
trap 'printf %s "$SAVED_CLIP" | pbcopy' EXIT

# Nimble is a non-activating panel: it takes keystrokes while another app stays frontmost, so
# "is it safe to type" comes from Nimble's own key-window log (run it with NIMBLE_DEBUG=1).
LOG="${NIMBLE_LOG:-/tmp/nimble.log}"
front() { osascript -e 'tell application "System Events" to get name of first process whose frontmost is true'; }
has_keys() { [ "$(rg -o 'panel key=(true|false)' "$LOG" | tail -1)" = "panel key=true" ]; }
shot() { screencapture -x "$OUT/$1.png"; sips -Z 1600 "$OUT/$1.png" --out "$OUT/$1.png" >/dev/null; echo "shot $1"; }
guard() {
  if ! has_keys; then
    echo "abort: nimble panel does not have keyboard focus (frontmost app: $(front))"
    exit 1
  fi
}
key() { guard; osascript -e "tell application \"System Events\" to key code $1"; sleep 0.4; }
type() { guard; osascript -e "tell application \"System Events\" to keystroke \"$1\""; sleep 0.6; }
combo() { guard; osascript -e "tell application \"System Events\" to keystroke \"$1\" using {$2}"; sleep 0.5; }
clip() { echo "clipboard: '$(pbpaste)'"; }

if ! has_keys; then
  osascript -e "tell application \"System Events\" to key code 49 using {${HOTKEY_MODS:-option down}}"
  sleep 0.8
fi
has_keys && echo "panel has keys (frontmost app: $(front))"
shot 1-shown

type "calc"
shot 2-typed-calc

printf 'sentinel' | pbcopy
combo a "command down"
combo c "command down"
clip
combo x "control down"
clip
shot 3-after-cut
combo v "command down"
combo v "control down"
shot 4-pasted-twice
combo z "command down"
shot 5-undo
combo z "command down, shift down"
shot 6-redo

combo a "command down"
key 51
type "term"
key 125
shot 7-arrow-down

key 126
type "inal"
shot 8-terminal

combo a "command down"
key 51
type "calculator"
key 36
sleep 1.2
echo "frontmost after enter: $(front); panel has keys: $(has_keys && echo yes || echo no)"
shot 9-launched
