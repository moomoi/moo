# Building and running

## Requirements

- macOS 14 or later, Apple Silicon (tested on macOS 26.6 arm64)
- Rust (tested with 1.94)
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
| `NIMBLE_HOTKEY` | Hotkey to try first, e.g. `ctrl+alt+space`. Defaults: `alt+space`, then `ctrl+alt+space`, then `cmd+shift+space` |
| `NIMBLE_PLUGINS` | Plugin folder |
| `NIMBLE_FRECENCY` | Frecency file (default `~/Library/Application Support/Nimble/frecency.tsv`) |
| `NIMBLE_DEBUG` | Timestamped logs on stderr: focus changes, hotkey, key handling, file query timings |

If the default hotkey does nothing, another app probably owns it: registration succeeds, but the
other app receives the keypress. Set `NIMBLE_HOTKEY`.

## Test

```sh
cd packages/nimble-macos && cargo test
```

Covers the Tier A sandbox (capabilities denied, `register` required, state kept across calls) and
frecency decay and persistence.

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
