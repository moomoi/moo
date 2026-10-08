# Building and running

## Requirements

- macOS 14 or later to run Moo, on Apple silicon or Intel (releases are universal). Building is
  tested on macOS 26.6 arm64.
- Rust (tested with 1.94); `rustup target add x86_64-apple-darwin` for universal builds
- Xcode or the Command Line Tools with the macOS 26 SDK: `build.rs` compiles
  `packages/moo-macos/swift/ai.swift` (Apple's on-device model) with `swiftc`. Tested with Swift 6.3.
- Node.js and npm. Everything else comes from package managers, nothing is vendored:
  - the tish compiler, `@tishlang/tish`, and Lattish, from npm (`package.json`; `npm ci`)
  - `@tishlang/tish-macos`, from npm (`app/package.json`; `app/build.sh` installs it)
  - the tish crates `packages/moo-macos` uses, from crates.io at the compiler's version, so the
    app, tish-macos and the generated code share one `tishlang_core`

  The scripts use `node_modules/.bin/tish`; override with `TISH=...`. To move to a new tish, bump
  `@tishlang/tish` in `package.json` and the `tishlang_*` versions in
  `packages/moo-macos/Cargo.toml` together.

## Get the code


```sh
git clone https://github.com/moomoi/moo.git
npm ci
```

Plugins are built and released in [moomoi/plugins](https://github.com/moomoi/plugins). Moo ships
the release named in `plugins.version`: `scripts/fetch-plugins.sh` downloads it into
`dist/plugins/` (checking its SHA256SUMS) when that version isn't there yet. A merged plugin ships
once its release is out and `plugins.version` names it. To try a plugins build first, point
`MOO_PLUGINS_TARBALL` at its `moo-plugins.tar.gz`.

## Build

```sh
npm ci                       # the tish compiler and Lattish (once, and after package.json changes)
npm run build                # plugins + app: dist/plugins/ (downloaded), app/dist/moo (fast compile)
npm run build:release        # the same, fully optimized like a release (several minutes from cold)
npm test                     # moo-macos unit tests
npm run clean                # remove every build output (dist/, target/, app and plugin node_modules)
```

Or step by step:

```sh
bash scripts/fetch-plugins.sh  # dist/plugins/: the moomoi/plugins release in plugins.version
bash app/build.sh            # app/dist/moo
bash scripts/bundle-macos.sh # both of the above, then dist/Moo.app
npm run build:web            # the moo.moi site with Orbit (see web.md)
```

`app/build.sh` installs the npm packages on first run (`npm ci` here and in `app/`). tish-macos,
the generated code and `packages/moo-macos` all use the tish crates from crates.io, so they share
one `tishlang_core`. Native builds share
`target/tish-native`, so rebuilds take about 40 s.

`bundle-macos.sh` produces an `LSUIElement` app (no Dock icon, bundle id `moi.moo.launcher`) with
plugins in `Contents/Resources/plugins`. It signs ad hoc; set `SIGN_IDENTITY` to sign with a
Developer ID, the hardened runtime and `packaging/entitlements.plist`. `SKIP_BUILD=1` reuses
existing builds; `VERSION` and `BUILD` set the bundle versions.

The icons come from one image, `packaging/icon.png` (1024×1024, full bleed). After changing it, run
`bash scripts/make-icons.sh` (needs ImageMagick) and commit what it writes:
`packaging/AppIcon.icns` (the app icon), `packaging/MenuBarIcon.tiff` (the menu bar template),
`packaging/SearchIcon.png` (the search field's icon) and
`web/public/icon.png` and `favicon.png` (the site's logo and favicon). Dev runs from `app/` load the icons from
`../packaging`; the bundle carries them in `Contents/Resources`.

Release builds (universal, notarized DMG) are `scripts/build-universal.sh` then
`scripts/package-release.sh`; see [release.md](release.md). `moo --version` prints the version a
release was built as (`dev` for local builds).

## Run

```sh
cd app && ./dist/moo          # from a build
open dist/Moo.app             # the bundle
```

| Variable | Effect |
| --- | --- |
| `MOO_HOTKEY` | Hotkey to try first, e.g. `ctrl+alt+space`. Defaults: `cmd+space`, then `alt+space`, `ctrl+alt+space`, `cmd+shift+space` |
| `MOO_PLUGINS` | Plugin folder |
| `MOO_FRECENCY` | Frecency file (default `~/Library/Application Support/Moo/frecency.tsv`) |
| `MOO_DEBUG` | Timestamped logs on stderr: focus changes, hotkey, key handling, file query timings |
| `MOO_CONFIG` | Shortcuts file (default `~/.config/moo/shortcuts.json`) |
| `MOO_SOCKET` | CLI socket (default `~/Library/Application Support/Moo/moo.sock`) |
| `MOO_START_HIDDEN` | `1`: start without showing the panel (set by the CLI when it starts Moo) |
| `MOO_SYSTEM_DRY_RUN` | Set: system commands (restart, lock, volume, …) only report what they would do |

The launcher hotkey is changed in Settings › Launcher Hotkey: press the new keys and Return. It
switches right away and is saved as `"launcher"` in `shortcuts.json` (which can also be edited by
hand; that takes effect at the next start). `MOO_HOTKEY` wins over it.

Hotkeys name the modifiers macOS receives, as in every other Mac app. If System Settings ›
Keyboard › Modifier Keys swaps Command and Control on a keyboard, pressing that keyboard's Control
key is ⌘ to macOS and to Moo; the hotkey recorder records the same thing, so what you press is
what you get.

Combinations an enabled macOS shortcut owns (Spotlight ⌘Space, input sources ⌃Space and ⌃⌥Space,
Finder search ⌘⌥Space, including their defaults when never changed) are skipped and the next
default is tried, because macOS accepts the registration and then never delivers the keypress.

If the default hotkey does nothing, another app probably owns it: registration succeeds, but the
other app receives the keypress. Set `MOO_HOTKEY`. Chrome's "Ask Gemini" bar also uses
option+space: Moo's panel opens and then loses focus to Chrome at once (the debug log shows
`panel key=true` followed by `panel key=false` within milliseconds).

## Command line

The app binary is the CLI. Link it onto your `PATH`:

```sh
ln -s "$PWD/dist/Moo.app/Contents/MacOS/moo" /usr/local/bin/moo   # or app/dist/moo
```

```sh
moo                                  # show the launcher (starts Moo if needed)
moo run g rust traits                # run a shortcut or command with text
moo run moo:clipboard             # open a built-in or plugin command (ids: moo list commands)
moo search invoice                   # show the launcher with text typed
moo files invoice -n 3 --json        # file paths from the live index
moo files 'quarterly report' --contents   # files whose text has these words
moo open-with ~/notes.txt            # apps that open it (* default); add a name to open with one
moo trash ~/old.txt                  # move to the Trash (prints where it went)
moo define serendipity               # senses, pronunciation and origin from the macOS dictionary
moo web how to make sourdough        # the default engine's suggestions
moo web engine duckduckgo            # pick the engine (no name: list them, * default)
moo contacts ada                     # matching contacts (after allowing access in Search Contacts)
moo apps safari                      # matching applications
moo ask "summarize: $(pbpaste)"      # on-device model; the answer streams
moo clipboard -n 5                   # clipboard history
moo shortcut add g url 'https://www.google.com/search?q={query}'
moo shortcut add proj open .         # relative paths resolve against your folder
moo shortcut add ip shell 'curl -s ifconfig.me' --output copy --hotkey ctrl+alt+i
moo shortcut add ';sig' text 'Best,\nA' --expand   # typing ;sig in any app becomes the text
moo shortcut rm ip
moo hotkey add cmd+shift+v moo:clipboard
moo hotkey add f5 g weather          # a hotkey can carry the text too
moo hotkey add ctrl+alt+s /Applications/Safari.app   # open an app
moo hotkey rm f5
moo list [shortcuts|commands|hotkeys] [--json]
moo status [--json]
moo config                           # path of shortcuts.json
moo help
```

Exit codes are 0 on success and 1 on any error, with the reason on stderr ("⌃⌥⇧F18 is already
bound to “Downloads”", "No shortcut or command called “x”"). A call to a running Moo takes
about 5 ms.

From other hotkey tools:

```sh
# skhd (~/.skhdrc)
cmd + shift - g : moo run g "$(pbpaste)"
alt - space : moo toggle
```

Karabiner-Elements, in a complex modification's `manipulators` (F13 opens clipboard history):

```json
{ "type": "basic", "from": { "key_code": "f13" },
  "to": [{ "shell_command": "/usr/local/bin/moo run moo:clipboard" }] }
```

BetterTouchTool, Hammerspoon (`hs.execute("moo run dl")`), Keyboard Maestro and Shortcuts.app
("Run Shell Script") work the same way.

## Test

```sh
cd packages/moo-macos && cargo test
```

Covers the Tier A sandbox (capabilities denied, `register` required, state kept across calls, time
budgets for runaway loops and recursion), frecency decay and persistence, the file index,
`shortcuts.json` parsing and templates, hotkey specs, shell output limits and a CLI socket round
trip.

The CLI also makes end-to-end tests possible without driving the UI. Run a separate instance
that cannot touch your real setup, then talk to it:

```sh
export MOO_SOCKET=/tmp/nt/s.sock MOO_CONFIG=/tmp/nt/shortcuts.json \
       MOO_FILE_INDEX=/tmp/nt/files.idx MOO_HOTKEY=ctrl+alt+shift+cmd+f17 \
       MOO_HISTORY=/tmp/nt/history.txt MOO_FRECENCY=/tmp/nt/frecency.tsv \
       MOO_PREFS=/tmp/nt/prefs.tsv MOO_SYSTEM_DRY_RUN=1
app/dist/moo shortcut add up shell 'echo up {query}'   # starts the instance hidden
app/dist/moo run up 'a b; touch /tmp/pwned'            # prints "up a b; touch /tmp/pwned"
app/dist/moo quit
```

Export these in the shell rather than prefixing one command: any `moo` command starts Moo
when none is running, and that instance inherits only the environment of the command that started
it. Without `MOO_SYSTEM_DRY_RUN=1`, Restart, Shut Down and the other system commands are real;
check with `moo system restart`, which must print `dry run: restart`, before pressing Return in
the panel. The clipboard is not isolated: AI answers that call `copyText` change it.

Window commands, `{selection}` and snippet expansion use Accessibility. macOS grants it to the
app that started the process: Moo started from a terminal or editor uses that app's
permission, and Moo started from Finder or at login needs its own entry in System Settings ›
Privacy & Security › Accessibility (the first window command, or the first expanding snippet,
asks). With `MOO_SYSTEM_DRY_RUN=1`, `moo window <layout>` prints the window's frame and
where it would go without moving it. A test instance watches for its own snippet keywords in
every app, so give test snippets keywords you would never type.

Two ignored tests act on real apps they open themselves, and skip when that app is already open:
`cargo test --lib -- --ignored acts_on_an_app_it_started` (launches and quits Chess) and
`replaces_a_keyword_in_textedit` (expands a keyword in a temporary file in TextEdit, then quits
it without saving). `finds_a_file_by_its_contents` writes a file into
`packages/moo-macos/spotlight-scratch/` (Spotlight skips `target/`), waits for Spotlight to
index it and removes it. `trash_moves_a_file_and_reports_where` is not ignored: unless
`MOO_SYSTEM_DRY_RUN` is set it moves a scratch file to the Trash and deletes it from there.

The `scripts/drive-*.sh` scripts drive a running Moo with System Events keystrokes and take
screenshots into `/tmp`. They need Accessibility permission for the terminal running them, expect
Moo started with `MOO_DEBUG=1` logging to `/tmp/moo.log`, refuse to type unless Moo's
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
shipped in `plugins/`). `drive-runaway.sh build` compiles it into `/tmp/moo-runaway/plugins`
next to `convert.tishc`; start Moo with `MOO_PLUGINS=/tmp/moo-runaway/plugins`.
