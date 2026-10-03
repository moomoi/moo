#!/usr/bin/env bash
# Exercise status item, clipboard history and frecency in a running Nimble (NIMBLE_DEBUG=1,
# ideally NIMBLE_FRECENCY pointing at a scratch file). Restores the clipboard on exit.
set -euo pipefail
OUT="${1:-/tmp/nimble-features}"
mkdir -p "$OUT"
LOG="${NIMBLE_LOG:-/tmp/nimble.log}"
MODS="${HOTKEY_MODS:-option down}"

SAVED_CLIP="$(pbpaste || true)"
trap 'printf %s "$SAVED_CLIP" | pbcopy' EXIT

has_keys() { [ "$(rg -o 'panel key=(true|false)' "$LOG" | tail -1)" = "panel key=true" ]; }
shot() { screencapture -x "$OUT/$1.png"; sips -Z 1600 "$OUT/$1.png" --out "$OUT/$1.png" >/dev/null; echo "shot $1"; }
guard() { has_keys || { echo "abort: panel does not have keys"; exit 1; }; }
key() { guard; osascript -e "tell application \"System Events\" to key code $1"; sleep 0.4; }
type() { guard; osascript -e "tell application \"System Events\" to keystroke \"$1\""; sleep 0.6; }
clear_query() { guard; osascript -e 'tell application "System Events" to keystroke "a" using {command down}'; key 51; }
summon() { has_keys || { osascript -e "tell application \"System Events\" to key code 49 using {$MODS}"; sleep 0.8; }; }
concealed_copy() {
  osascript -e 'use framework "AppKit"' -e 'set pb to current application'"'"'s NSPasteboard'"'"'s generalPasteboard()' \
    -e 'pb'"'"'s clearContents()' -e "pb's setString:\"$1\" forType:\"public.utf8-plain-text\"" \
    -e 'pb'"'"'s setString:"" forType:"org.nspasteboard.ConcealedType"' >/dev/null
}

echo "== status item"
osascript -e 'tell application "System Events" to tell process "nimble" to click menu bar item 1 of menu bar 2'
sleep 0.6
shot 1-status-menu
osascript -e 'tell application "System Events" to tell process "nimble" to click menu item 1 of menu 1 of menu bar item 1 of menu bar 2'
sleep 0.8
has_keys && echo "panel shown from status menu"

echo "== clipboard history"
osascript -e "tell application \"System Events\" to key code 53"; sleep 0.5
printf 'first clip: hello from the clipboard' | pbcopy; sleep 0.7
printf 'https://tishlang.com/docs' | pbcopy; sleep 0.7
concealed_copy "hunter2-secret-password"; sleep 0.7
printf 'SELECT * FROM launches WHERE app = %s' "'nimble'" | pbcopy; sleep 0.7
summon
clear_query
type "clip"
shot 2-root-clip
key 36
sleep 0.4
shot 3-clipboard-list
type "tish"
shot 4-clipboard-filtered
key 36
sleep 0.5
echo "clipboard after enter: '$(pbpaste)'"
rg -q "hunter2" "$LOG" && echo "LEAK: concealed text in log" || echo "concealed copy not in log"

echo "== frecency"
summon
clear_query
type "c"
shot 5-c-before
for _ in 1 2 3; do
  clear_query
  type "calculator"
  key 36
  sleep 1.0
  osascript -e 'tell application "Calculator" to quit' || true
  sleep 0.5
  summon
done
clear_query
type "c"
shot 6-c-after
clear_query
shot 7-empty-recents
key 53
