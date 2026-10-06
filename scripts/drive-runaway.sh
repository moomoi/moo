#!/usr/bin/env bash
# Exercise the Tier A call budget with a deliberately hanging plugin (scripts/fixtures/runaway.tish).
#   bash scripts/drive-runaway.sh build    # /tmp/moo-runaway/plugins: runaway.tishc + convert.tishc
#   MOO_DEBUG=1 MOO_PLUGINS=/tmp/moo-runaway/plugins app/dist/moo 2>/tmp/moo.log &
#   bash scripts/drive-runaway.sh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
if [ "${1:-}" = "build" ]; then
  TISH="${TISH:-$ROOT/node_modules/.bin/tish}"
  mkdir -p /tmp/moo-runaway/plugins
  "$TISH" build "$ROOT/scripts/fixtures/runaway.tish" --target bytecode -o /tmp/moo-runaway/plugins/runaway.tishc
  cp "$ROOT/plugins/dist/convert.tishc" /tmp/moo-runaway/plugins/
  exit 0
fi
OUT="${1:-/tmp/moo-runaway/shots}"
mkdir -p "$OUT"
LOG="${MOO_LOG:-/tmp/moo.log}"

has_keys() { [ "$(rg -o 'panel key=(true|false)' "$LOG" | tail -1)" = "panel key=true" ]; }
shot() { screencapture -x "$OUT/$1.png"; sips -Z 1600 "$OUT/$1.png" --out "$OUT/$1.png" >/dev/null; echo "shot $1"; }
guard() { has_keys || { echo "abort: moo panel does not have keyboard focus"; exit 1; }; }
type() { guard; osascript -e "tell application \"System Events\" to keystroke \"$1\""; sleep "${2:-0.7}"; }
key() { guard; osascript -e "tell application \"System Events\" to key code $1"; sleep "${2:-0.5}"; }
stamp() { python3 -c 'import time; print(f"{time.time():.3f}")'; }

if ! has_keys; then
  osascript -e "tell application \"System Events\" to key code 49 using {${HOTKEY_MODS:-option down}}"
  sleep 0.8
fi

type "runaway echo"
key 36
type "hi"
shot 1-echo-hi
osascript -e 'tell application "System Events" to keystroke "a" using {command down}'
t0="$(stamp)"
type "spin" 1.0
echo "typed spin at $t0"
shot 2-echo-spin
osascript -e 'tell application "System Events" to keystroke "a" using {command down}'
type "ok"
shot 3-echo-ok-after
key 53 0.3
osascript -e 'tell application "System Events" to keystroke "a" using {command down}'
type "runaway hang"
key 36 1.0
shot 4-hang
osascript -e 'tell application "System Events" to keystroke "a" using {command down}'
type "convert"
key 36
type "10 km"
shot 5-convert-still-works
key 53
key 53
