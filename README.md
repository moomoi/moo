<p align="center">
  <img src="assets/icon.png" width="128" height="128" alt="Moo">
</p>

<h1 align="center">Moo</h1>

<p align="center">
  <b>One hotkey. Then type anything.</b><br>
  A fast, native launcher for macOS.
</p>

<p align="center">
  <a href="https://github.com/moomoi/moo/releases/latest/download/Moo-macos-universal.dmg"><b>Download for Mac</b></a>
  ·
  <a href="https://moo.moi">moo.moi</a>
  ·
  <a href="https://moo.moi/docs">Docs</a>
  ·
  <a href="https://moo.moi/marketplace">Marketplace</a>
</p>

---

Moo is a keyboard launcher for macOS. Apps, files, the clipboard, calculations, window layouts,
system commands, Ask AI and plugins all live in one search field. It's one native AppKit process
built with [Tish](https://github.com/tishlang/tish), with no webviews, no Electron and no
JavaScript, so it opens instantly and stays out of the way.

This repository holds Moo's releases. The documentation is at [moo.moi/docs](https://moo.moi/docs).

## What it does

- **Apps and files.** Fuzzy search over every app, and files from Moo's own live index. Search
  inside documents too, with Quick Look, Show in Finder and Open With one key away.
- **Instant answers.** Math, percentages, units, currencies (ECB rates), number bases and time
  zones appear as you type. `12% of 3400`, `5 mi in km`, `3pm pst to cet`.
- **Clipboard history.** Everything you copied, searchable. Text only, kept in memory, never
  written to disk. Password managers' entries are skipped.
- **Window layouts.** Halves, quarters, thirds, maximize, center, and moving to another display.
  They act on the window you were just using.
- **System commands.** Lock, sleep, dark mode, volume, eject, empty Trash, restart and more.
- **Shortcuts and snippets.** Your own keywords for URLs, folders, shell commands, text and AI
  prompts, each with an optional global hotkey. Snippets expand as you type in any app.
- **Ask AI.** Press <kbd>Tab</kbd> on any question. Apple's on-device model is the default: free,
  offline and private. It can find files, check your battery, arrange windows and use the
  calculator. Hypery, OpenAI, Ollama, LM Studio and any OpenAI-compatible server work too.
- **Plugins.** Written in Tish. Each one runs in its own sandboxed VM with no file or network
  access beyond the hosts it declares. Slack, Unit Converter and Moo Utils ship built in. Browse
  them in the [marketplace](https://moo.moi/marketplace).
- **Command line.** The app is also the `moo` CLI, so you can script it or drive it from skhd,
  Karabiner, Hammerspoon or Shortcuts.

## Install

**Download** [Moo-macos-universal.dmg](https://github.com/moomoi/moo/releases/latest/download/Moo-macos-universal.dmg),
open it and drag Moo to Applications.

**Or with Homebrew:**

```sh
brew install moomoi/moo/moo
```

Requires macOS 14 or later, on Apple silicon or Intel. Ask AI on Apple's on-device model needs
macOS 26 with Apple Intelligence turned on; other models work everywhere.

Every release is signed with a Developer ID and notarized by Apple. To check a download:

```sh
shasum -a 256 -c SHA256SUMS
spctl -a -vv -t exec /Applications/Moo.app   # source=Notarized Developer ID
```

## First run

Moo lives in the menu bar. Press the hotkey it shows (<kbd>⌘ Space</kbd> if Spotlight's is off,
otherwise the first free one of <kbd>⌥ Space</kbd>, <kbd>⌃ ⌥ Space</kbd>, <kbd>⌘ ⇧ Space</kbd>)
and type. Change it in Settings (<kbd>⌘ ,</kbd>).

macOS asks once for each permission when a feature first needs it: Accessibility (window
layouts, selected text, snippets) and Contacts (contact search).

New to Moo? Start with [Getting started](https://moo.moi/docs/install).

## Privacy

No analytics, no telemetry. Your searches, clipboard and settings stay on your Mac, and keys and
tokens go in your Keychain. Moo only goes online for what you ask: the AI provider you choose,
currency rates, web suggestions and plugins you use. Details are at
[moo.moi/legal/privacy](https://moo.moi/legal/privacy).

## Updating

Homebrew: `brew upgrade moomoi/moo/moo`. Otherwise download the latest release over the old app.

## Feedback

Found a bug or want a feature? [Open an issue](https://github.com/moomoi/moo/issues).

## License

[MIT](LICENSE)
