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

The hotkey is registered exactly as written (`cmd+space` registers Command+Space). System
Settings › Keyboard › Modifier Keys swaps are applied by macOS before the keypress reaches Nimble,
so Nimble does nothing about them.

Combinations an enabled macOS shortcut owns (Spotlight ⌘Space, input sources ⌃Space and ⌃⌥Space,
Finder search ⌘⌥Space, including their defaults when never changed) are skipped and the next
default is tried, because macOS accepts the registration and then never delivers the keypress.

If the default hotkey does nothing, another app probably owns it: registration succeeds, but the
other app receives the keypress. Set `NIMBLE_HOTKEY`. Chrome's "Ask Gemini" bar also uses
option+space: Nimble's panel opens and then loses focus to Chrome at once (the debug log shows
`panel key=true` followed by `panel key=false` within milliseconds).

## Test

```sh
cd packages/nimble-macos && cargo test
```

Covers the Tier A sandbox (capabilities denied, `register` required, state kept across calls, time
budgets for runaway loops and recursion) and frecency decay and persistence.

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
