# Building and running

## Requirements

- macOS 14 or later, Apple Silicon (tested on macOS 26.6 arm64)
- Rust (tested with 1.94)
- Xcode or the Command Line Tools with the macOS 26 SDK: `build.rs` compiles
  `packages/nimble-macos/swift/ai.swift` (Apple's on-device model) with `swiftc`. Tested with Swift 6.3.
- A tish compiler built from the `tish-nimble` checkout, which carries the cdylib and runtime
  module-loading changes: `cargo build --release -p tishlang --features full` in
  `~/Projects/tish/tish-nimble`. The scripts default to
  `~/Projects/tish/tish-nimble/target/release/tish`; override with `TISH=...`.

## Build

```sh
bash plugins/build.sh        # plugins/dist/convert.tishc, plugins/dist/utils.lib
bash app/build.sh            # app/dist/nimble
bash scripts/bundle-macos.sh # both of the above, then dist/Nimble.app
```

`app/build.sh` vendors tish-macos on first run (`scripts/vendor-tish-apple.sh`) so that it, the
compiler runtime and `packages/nimble-macos` all use the same `tishlang_core`. Native builds share
`target/tish-native`, so rebuilds take about 40 s.

`bundle-macos.sh` produces an `LSUIElement` app (no Dock icon) with plugins in
`Contents/Resources/plugins`. It signs ad hoc; set `SIGN_IDENTITY` to sign with a Developer ID and
the hardened runtime. `SKIP_BUILD=1` reuses existing builds.

## Run

```sh
cd app && ./dist/nimble          # from a build
open dist/Nimble.app             # the bundle
```

| Variable | Effect |
| --- | --- |
| `NIMBLE_HOTKEY` | Hotkey to try first, e.g. `ctrl+alt+space`. Defaults: `cmd+space`, then `alt+space`, `ctrl+alt+space`, `cmd+shift+space` |
| `NIMBLE_PLUGINS` | Plugin folder |
| `NIMBLE_FRECENCY` | Frecency file (default `~/Library/Application Support/Nimble/frecency.tsv`) |
| `NIMBLE_DEBUG` | Timestamped logs on stderr: focus changes, hotkey, key handling, file query timings |
| `NIMBLE_CONFIG` | Shortcuts file (default `~/.config/nimble/shortcuts.json`) |
| `NIMBLE_SOCKET` | CLI socket (default `~/Library/Application Support/Nimble/nimble.sock`) |
| `NIMBLE_START_HIDDEN` | `1`: start without showing the panel (set by the CLI when it starts Nimble) |
| `NIMBLE_SYSTEM_DRY_RUN` | Set: system commands (restart, lock, volume, …) only report what they would do |

The launcher hotkey can also be set as `"launcher"` in `shortcuts.json`; `NIMBLE_HOTKEY` wins
over it.

Hotkeys name keys as printed on the keyboard. System Settings › Keyboard › Modifier Keys remaps
them per keyboard below the event system (with Command and Control swapped, the Command key sends
Control), so `keymap.rs` reads each connected keyboard's mapping
(`com.apple.keyboard.modifiermapping.<vendor>-<product>-0` in the current-host global domain) and
registers what the keys actually produce: `cmd+space` becomes `ctrl+space` on a swapped keyboard,
and keyboards or left/right keys that differ get one registration each. The debug log shows it:
`hotkey ⌘Space registered as ctrl+space`. Mappings are read when the hotkey is registered, so
restart Nimble after changing them.

Combinations an enabled macOS shortcut owns (Spotlight ⌘Space, input sources ⌃Space and ⌃⌥Space,
Finder search ⌘⌥Space, including their defaults when never changed) are skipped and the next
default is tried, because macOS accepts the registration and then never delivers the keypress.

If the default hotkey does nothing, another app probably owns it: registration succeeds, but the
other app receives the keypress. Set `NIMBLE_HOTKEY`. Chrome's "Ask Gemini" bar also uses
option+space: Nimble's panel opens and then loses focus to Chrome at once (the debug log shows
`panel key=true` followed by `panel key=false` within milliseconds).

## Command line

The app binary is the CLI. Link it onto your `PATH`:

```sh
ln -s "$PWD/dist/Nimble.app/Contents/MacOS/nimble" /usr/local/bin/nimble   # or app/dist/nimble
```

```sh
nimble                                  # show the launcher (starts Nimble if needed)
nimble run g rust traits                # run a shortcut or command with text
nimble run nimble:clipboard             # open a built-in or plugin command (ids: nimble list commands)
nimble search invoice                   # show the launcher with text typed
nimble files invoice -n 3 --json        # file paths from the live index
nimble apps safari                      # matching applications
nimble ask "summarize: $(pbpaste)"      # on-device model; the answer streams
nimble clipboard -n 5                   # clipboard history
nimble shortcut add g url 'https://www.google.com/search?q={query}'
nimble shortcut add proj open .         # relative paths resolve against your folder
nimble shortcut add ip shell 'curl -s ifconfig.me' --output copy --hotkey ctrl+alt+i
nimble shortcut rm ip
nimble hotkey add cmd+shift+v nimble:clipboard
nimble hotkey add f5 g weather          # a hotkey can carry the text too
nimble hotkey rm f5
nimble list [shortcuts|commands|hotkeys] [--json]
nimble status [--json]
nimble config                           # path of shortcuts.json
nimble help
```

Exit codes are 0 on success and 1 on any error, with the reason on stderr ("⌃⌥⇧F18 is already
bound to “Downloads”", "No shortcut or command called “x”"). A call to a running Nimble takes
about 5 ms.

From other hotkey tools:

```sh
# skhd (~/.skhdrc)
cmd + shift - g : nimble run g "$(pbpaste)"
alt - space : nimble toggle
```

Karabiner-Elements, in a complex modification's `manipulators` (F13 opens clipboard history):

```json
{ "type": "basic", "from": { "key_code": "f13" },
  "to": [{ "shell_command": "/usr/local/bin/nimble run nimble:clipboard" }] }
```

BetterTouchTool, Hammerspoon (`hs.execute("nimble run dl")`), Keyboard Maestro and Shortcuts.app
("Run Shell Script") work the same way.

## Test

```sh
cd packages/nimble-macos && cargo test
```

Covers the Tier A sandbox (capabilities denied, `register` required, state kept across calls, time
budgets for runaway loops and recursion), frecency decay and persistence, the file index,
`shortcuts.json` parsing and templates, hotkey specs, shell output limits and a CLI socket round
trip.

The CLI also makes end-to-end tests possible without driving the UI. Run a separate instance
that cannot touch your real setup, then talk to it:

```sh
export NIMBLE_SOCKET=/tmp/nt/s.sock NIMBLE_CONFIG=/tmp/nt/shortcuts.json \
       NIMBLE_FILE_INDEX=/tmp/nt/files.idx NIMBLE_HOTKEY=ctrl+alt+shift+cmd+f17 \
       NIMBLE_HISTORY=/tmp/nt/history.txt NIMBLE_FRECENCY=/tmp/nt/frecency.tsv \
       NIMBLE_PREFS=/tmp/nt/prefs.tsv NIMBLE_SYSTEM_DRY_RUN=1
app/dist/nimble shortcut add up shell 'echo up {query}'   # starts the instance hidden
app/dist/nimble run up 'a b; touch /tmp/pwned'            # prints "up a b; touch /tmp/pwned"
app/dist/nimble quit
```

Export these in the shell rather than prefixing one command: any `nimble` command starts Nimble
when none is running, and that instance inherits only the environment of the command that started
it. Without `NIMBLE_SYSTEM_DRY_RUN=1`, Restart, Shut Down and the other system commands are real;
check with `nimble system restart`, which must print `dry run: restart`, before pressing Return in
the panel. The clipboard is not isolated: AI answers that call `copyText` change it.

Window commands and `{selection}` use Accessibility. macOS grants it to the app that started the
process: Nimble started from a terminal or editor uses that app's permission, and Nimble started
from Finder or at login needs its own entry in System Settings › Privacy & Security ›
Accessibility (the first window command asks). With `NIMBLE_SYSTEM_DRY_RUN=1`, `nimble window
<layout>` prints the window's frame and where it would go without moving it.

The `scripts/drive-*.sh` scripts drive a running Nimble with System Events keystrokes and take
screenshots into `/tmp`. They need Accessibility permission for the terminal running them, expect
Nimble started with `NIMBLE_DEBUG=1` logging to `/tmp/nimble.log`, refuse to type unless Nimble's
panel has keyboard focus, and restore the clipboard on exit. Set
`HOTKEY_MODS="control down, option down"` when using `ctrl+alt+space`.

| Script | Covers |
| --- | --- |
| `drive-demo.sh` | Hotkey, typing, edit shortcuts, arrows, launching an app |
| `drive-files.sh` | Spotlight file results |
| `drive-plugins.sh` | Tier B plugin: no-view and list commands |
| `drive-convert.sh` | Tier A plugin: list command, VM state |
| `drive-features.sh` | Status item, clipboard history, frecency |
| `drive-runaway.sh` | Tier A time budget: a hanging plugin errors, other plugins keep working |
| `drive-ai.sh` | Ask AI: tab from root, follow-up, copy, scrolling, esc to stop (needs Apple Intelligence on) |

`drive-runaway.sh` uses a plugin that hangs on purpose (`scripts/fixtures/runaway.tish`, not
shipped in `plugins/`). `drive-runaway.sh build` compiles it into `/tmp/nimble-runaway/plugins`
next to `convert.tishc`; start Nimble with `NIMBLE_PLUGINS=/tmp/nimble-runaway/plugins`.
