---
title: Install
summary: Get moo onto your Mac in under a minute, from the download or Homebrew.
---
## Requirements

- macOS 14 Sonoma or later
- Apple silicon or Intel. Every release is a universal build.
- Ask AI on Apple's on-device model needs macOS 26 with Apple Intelligence turned on. Everything else works without it.

## Download

1. Download the latest release from [moo.moi/get](/get). It's a signed, notarized `.dmg`.
2. Open it and drag **Moo** into **Applications**.
3. Open Moo from Applications. It lives in the menu bar, not the Dock.

## Homebrew

```sh
brew install moomoi/moo/moo
```

Updates come with `brew upgrade`.

> **Tip** Every release is also on [GitHub](https://github.com/moomoi/moo/releases), with a zip of the app and `SHA256SUMS` alongside the DMG.

## Uninstall

Quit moo (type `quit moo`, or right-click its menu bar icon and choose **Quit**) and drag it from Applications to the Trash. To remove your settings too, delete `~/.config/moo` and `~/Library/Application Support/Moo`.
