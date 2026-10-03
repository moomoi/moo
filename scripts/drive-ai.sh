#!/usr/bin/env bash
# Exercise Ask AI (Apple's on-device model) in a running Nimble (NIMBLE_DEBUG=1): tab from root,
# a follow-up in the same session, copy, and esc to stop a streaming reply.
set -euo pipefail
OUT="${1:-/tmp/nimble-ai}"
mkdir -p "$OUT"
LOG="${NIMBLE_LOG:-/tmp/nimble.log}"
SAVED_CLIP="$(pbpaste || true)"
trap 'printf %s "$SAVED_CLIP" | pbcopy' EXIT

has_keys() { [ "$(rg -o 'panel key=(true|false)' "$LOG" | tail -1)" = "panel key=true" ]; }
shot() { screencapture -x "$OUT/$1.png"; sips -Z 1600 "$OUT/$1.png" --out "$OUT/$1.png" >/dev/null; echo "shot $1"; }
guard() { has_keys || { echo "abort: nimble panel does not have keyboard focus"; exit 1; }; }
type() { guard; osascript -e "tell application \"System Events\" to keystroke \"$1\""; sleep "${2:-0.7}"; }
key() { guard; osascript -e "tell application \"System Events\" to key code $1"; sleep "${2:-0.5}"; }

if ! has_keys; then
  osascript -e "tell application \"System Events\" to key code 49 using {${HOTKEY_MODS:-option down}}"
  sleep 0.8
fi

type "how many feet are in a mile" 0.4
shot 1-root-fallback
key 48 0.3
shot 2-streaming
sleep 4
shot 3-answer
type "and in a kilometer" 0.3
key 36 5
shot 4-follow-up
key 36 0.4
echo "clipboard: '$(pbpaste | head -c 200)'"
type "list ten tips for packing light, one sentence each" 0.3
key 36 8
shot 5-long-answer
key 125 0.2
key 125 0.4
shot 6-scrolled
type "write a long story about a lighthouse keeper" 0.3
key 36 1.2
key 53 0.8
shot 7-stopped
key 53
shot 8-back-at-root
key 53
