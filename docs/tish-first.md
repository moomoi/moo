# One language: moving Moo's Rust to Tish

Moo is a Tish app with a 15,600-line Rust crate (`packages/moo-macos`, the `tish:moo` native
module) and two Swift files. This plan moves everything that can be Tish into Tish, moves generic
macOS APIs into tish-macos (tishlang/tish-apple), and leaves Moo with as little native code of its
own as possible. Moo has no users yet, so nothing here needs to be backwards compatible: settings
files, stored data and APIs can change shape freely, and migration code can be deleted.

## Where each piece goes

| Destination | What |
|---|---|
| **Moo, in Tish** | shortcuts and `shortcuts.json`; calculator and currency; prefs, recent searches, frecency; saved chats; AI providers (Hypery, OpenAI, Ollama, LM Studio: credentials, streaming, retry, models, accounts); OAuth/PKCE logic; window-layout geometry; hotkey key tables and conflict checks; dictionary entry parsing; clipboard history; site-icon cache; theme parsing; Spotlight query building and ranking; CLI command handling; time-zone parsing |
| **tish-macos (upstream)** | non-activating panel and its animation; key, hotkey and scroll monitors; status item and menu; pasteboard; Accessibility (window frames, selected text, typing); system actions (lock, sleep, dark mode, volume, eject, quit apps); Contacts; DictionaryServices lookup; Keychain get/set/delete; FSEvents; MDQuery; Quick Look; app icons and NSWorkspace open/reveal; URL scheme handler; time-zone offsets; FoundationModels (Swift); **main-thread async primitives** (below) |
| **Moo, native (stays)** | the plugin VM embedder and its host object (`vmplug.rs`, the core of `pluginhost.rs`): identity, network allow-list and quotas must be enforced outside the plugin VM; the file-name index (`fsindex.rs`, `fslive.rs`): a per-keystroke scan of 100k–1M names needs compact columns and threads Tish doesn't have. Its skip lists, score weights and final rerank move to Tish. |

### The blocker: async on the main thread

`tish:http` streams (`fetch(url)` → `body.getReader()` → `await reader.read()` yields byte
chunks). But on the native backend `await` blocks the OS thread, and Moo's UI runs on the AppKit
main thread, so a Tish script there can't await a stream, a timer or a slow request without
freezing the launcher.

Running Tish code on a background thread is not an option either: the compiler keeps module
functions and numeric globals in per-thread storage (it only turns that off for `serve` handlers),
so a function run elsewhere sees empty globals. So Tish code stays on the main thread, and
tish-macos waits for it instead:

- `macos.whenSettled(promise, cb)`: waits for any Tish promise (`fetch`, `res.text()`,
  `reader.read()`) on a background thread and calls `cb(value, error)` on the main thread.
- `macos.startTimers()`: drives `setTimeout` / `setInterval` from the main run loop (`macos.run`
  already does; Moo uses `macos.run`).

Moo wraps callbacks in its `withUi` native so a re-render keeps the search field's focus.

## Phases

### Phase 1: fix what's broken (now)

Security, from the review:

1. Plugin identity comes from the installed file name, not the plugin's own `manifest().id`, so a
   plugin can't claim another's Keychain secrets and store (`vmplug.rs:79`).
2. Plugin fetches don't follow redirects off the declared hosts (`pluginhost.rs:196`, http.swift).
3. Plugin secret accounts can't collide: ids and keys are validated (`pluginhost.rs:155`).
4. Each sign-in has its own cancel, so one plugin can't cancel another's (`pluginhost.rs:37`).
5. OAuth: the `state` check comes before honouring `error=`, a mismatched callback is ignored rather
   than ending the sign-in, an empty `code` is rejected, and a failed `getentropy` fails the
   sign-in instead of producing an all-zero verifier (`oauth.rs:74, 275-284`).
6. Shell shortcuts pass `{query}`, `{selection}` and `{clipboard}` as arguments (`"$1"`), never
   spliced into the script, so a template that quotes a placeholder can't be escaped
   (`shortcuts.rs`, Encoding::Shell).

Crashes and hangs:

7. The calculator has a nesting limit (`calc.rs`: `((((…` / `----…` overflowed the stack).
8. Deleting a folder no longer scans the whole index once per entry (`fsindex.rs:620`).
9. A shell shortcut that times out kills its whole process group, so pipelines don't leak a
   thread (`shell.rs:76`).
10. The CLI server has read/write timeouts and doesn't write while holding the global lock
    (`cli.rs`).

Correctness:

11. OAuth refresh is single-flight per account, so two requests can't race a rotating refresh
    token and sign the user out (`remote.rs:217`).
12. SSE keeps blank lines and joins multi-line `data:` (http.swift `.lines`, `remote.rs:307`).
13. Tool calls and plugin `moo.log` don't write user content or tokens to the log (`ai.rs:119`,
    `pluginhost.rs:385`).
14. Theme colours resolve through an allowlist, not an arbitrary `performSelector`
    (`theme.rs:212`).

Fixes 11–13 land with the Tish ports of that code where that's sooner.

### Phase 2: port pure logic to Tish

No new native APIs needed. Each port deletes its Rust and its `lib.rs` exports.

- `shortcuts.rs` → `app/src/shortcuts.tish`. The file is read and written as JSON with unknown
  entries kept (fixes the data loss on save).
- `calc.rs` (+ `parse_ecb`) → `calc.tish`, with a nesting limit.
- `prefs.rs`, `history.rs`, `frecency.rs`, `chats.rs` → Tish over `tish:fs`. Drop the legacy
  "Nimble" migrations.
- `layout.rs`, `keys.rs`, the key-name table in `mac.rs`, `keymap::conflict` → Tish.
- Pure parts of `dict.rs`, `clip.rs`, `theme.rs`, `tz.rs`, `files.rs` (query building, ranking),
  `index.rs` (app scanning), `cli.rs` (dispatch and usage) → Tish, behind the thin native calls
  they keep.

### Phase 3: tish-macos

`macos.whenSettled` and `macos.startTimers` (tishlang/tish-apple#15). Importing `tish:http` turns
on `send-values`, which exposed two tish bugs, fixed in tishlang/tish#753: reading the same
variable twice in one statement deadlocked, and `JSON.parse` dropped `\uD83D\uDE00`-style
surrogate pairs (emoji from Python-based servers).

### Phase 4: port what needed async

`rates.rs`, `siteicon.rs` (all but naming an image file), `remote.rs`, `websearch.rs` → Tish on
`macos.whenSettled` and `tish:http`. `swift/http.swift` is deleted; `http.rs` is a small blocking
reqwest client for the plugin host and OAuth token requests (no redirects unless allowed).

Staying native for now:

- `oauth.rs`: the plugin host's sign-in uses it, so the app's AI sign-in and token refresh call it
  too (`oauthSignIn`, `oauthRefresh`): token requests must not follow redirects, which
  `tish:http` can't turn off.
- `keychain.rs`: plugins' secrets; the app reaches it through `keychain*` natives.
- `shell.rs`: needs a process group it can kill on timeout; `tish:process` has no async run.
- `files.rs` (Spotlight) and `bridge.rs`: move with the bindings in phase 5.

### Phase 5: move the remaining bindings

Switch Moo from its own bindings to tish-macos's and delete them from `moo-macos`. What's left is
the plugin VM host and the file index.

## Tracking

Each phase lands as PRs against `main`; this file is updated as items land.

- Phase 1: fixes 1–10, 13 and 14 done. 11 (refresh race) and 12 (SSE blank lines) land with the
  phase 4 port of `remote.rs` and the HTTP layer, which replaces that code.
- Phase 2: the calculator is Tish (`app/src/calc.tish`, tests in `calc.test.tish`, run with
  `tish test`). Native keeps `currencyRates()` (until `rates.rs` moves in phase 4) and
  `timeAnswer()` (until `tz.rs` moves).
- Phase 2: shortcuts.json is Tish (`app/src/shortcuts.tish`, pure and tested; file access in
  `shortcutsfile.tish`). Unknown keys in the file are kept on save. Native keeps `keysError()`
  (until `keys.rs` moves), `localDateTime()` and `watchShortcuts(dir, cb)`; `remote.rs` reads the
  `ai` section itself until it moves in phase 4.
- Phase 2: prefs and search history are Tish (`app/src/stores.tish`); `chats.rs` was unused and is
  deleted, as is the Nimble data-folder migration. Frecency stays native for now: the native app
  and file index ranks with it on every keystroke.
- Phase 2: time-zone answers are Tish (`app/src/tz.tish`); native keeps only macOS's zone database
  (`zoneNames`, `zoneLocal`, `zoneAt`, `zoneByAbbreviation` in `tzdb.rs`), which moves to
  tish-macos in phase 3. `layout.rs`, `keys.rs` and `keymap.rs` are pure but only native code calls
  them (hotkey registration, key recording, the panel key monitor, window arranging), so they move
  with those callers in phases 3 and 5 rather than now.
- Phase 2: clipboard history (`clip.tish`; native only watches the pasteboard), dictionary entry
  parsing (`dict.tish`; native only looks words up) and the CLI help text are Tish. Staying for
  later phases, because only native code calls them or they run on native threads: `theme.rs`
  (turns the theme into AppKit colours for the panel), `files.rs` (the Spotlight driver, phase 4),
  `index.rs` (app scan and nucleo fuzzy matching on the per-keystroke path), `cli.rs` (the socket).
  Phase 2 is done.
- Phase 3: tish-macos `whenSettled` / `startTimers` (tish-apple#15) and the tish fixes (tish#753).
  Moo needs both released (tish-macos 1.5.0, tish 3.15.3).
- Phase 4: currency rates, site icons, AI providers (with fixes 11 and 12) and web suggestions are
  Tish; `http.swift` is gone.
- Phase 5, first batch: time zones, the dictionary, the pasteboard, opening/revealing/trashing
  files, folder watching and the `moo://` handler come from tish-macos (tish-apple#16);
  `tzdb.rs`, `dict.rs`, `clip.rs`, `fileops.rs` and `watch.rs` are deleted. `app/src/platform.tish`
  adapts them to the launcher (hide after opening, "Moo" for its own copies, dry runs).
- Phase 5, second batch: system actions, running apps, system info and Contacts come from
  tish-macos; `system.rs`, `sysinfo.rs` and `contacts.rs` are deleted. The command dispatcher
  (messages, dry runs, on/off/toggle, volume steps) is Tish in `platform.tish`.
- Phase 5, third batch: Accessibility (selected text, snippet replacement, window frames) and the
  display list come from tish-macos; window layout geometry is Tish (`windowlayout.tish`, tests
  ported). `ax.rs` and `layout.rs` are deleted. Watching typed keywords stays native with the key
  monitors.
- Phase 5, fourth batch: Spotlight comes from tish-macos (`macos.spotlight.query`); query
  building, filtering and ranking are Tish (`spotlightquery.tish`, tests ported, and
  `platform.tish`). `files.rs` is deleted.
- Phase 5, fifth batch: icons and the menu bar item come from tish-macos; `siteicon.rs` and the
  icon and status-item code in `mac.rs` are deleted. Staying in Moo: the launcher panel (its
  shape, animation, theme and key routing), the hotkeys and key monitors that share its key
  handling, Quick Look over the panel, and Apple's on-device model (`ai.swift` needs the macOS 26
  SDK, which tish-macos shouldn't force on every app). Phase 5 is done.
