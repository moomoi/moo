# Moo

A keyboard launcher for macOS: apps, files, the clipboard, calculations, window layouts, system
commands, Ask AI and plugins, from one search field. Moo is one native process. No webviews, no
Electron, no JavaScript.

This repository holds Moo's releases. More at [moo.moi](https://moo.moi).

## Install

**Download** [Moo-macos-universal.dmg](https://github.com/moomoi/moo/releases/latest/download/Moo-macos-universal.dmg),
open it and drag Moo to Applications.

**Or with Homebrew:**

```sh
brew install --cask moomoi/moo/moo
```

Requires macOS 14 or later, on Apple silicon or Intel. Apple Intelligence answers in Ask AI need
Apple silicon with Apple Intelligence turned on; other models work everywhere.

Every release is signed with a Developer ID and notarized by Apple. To check a download:

```sh
shasum -a 256 -c SHA256SUMS
spctl -a -vv -t exec /Applications/Moo.app   # source=Notarized Developer ID
```

## First run

Press the hotkey Moo shows on first launch, then type. macOS asks once for each permission when a
feature first needs it: Accessibility (window layouts, selected text, snippets) and Contacts
(contact search).

## Updating

Homebrew: `brew upgrade --cask moo`. Otherwise download the latest release over the old app.

## License

[MIT](LICENSE)
