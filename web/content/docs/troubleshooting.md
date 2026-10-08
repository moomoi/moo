---
title: Troubleshooting
description: The usual suspects, and how to fix them.
slug: troubleshooting
section: Help
order: 22
---
## The hotkey does nothing

Another app owns the combination: moo registers it, but the keypress goes to the other app. Choose another hotkey in Settings, or set `MOO_HOTKEY`. See [The hotkey](/docs/hotkey).

## The launcher opens, then disappears

Something took focus straight away. Chrome's "Ask Gemini" bar on <kbd>⌥</kbd> <kbd>Space</kbd> is the common one. Change either hotkey.

## Window commands don't work

moo needs Accessibility permission: **System Settings › Privacy & Security › Accessibility**. If moo was started from a terminal, the terminal needs it instead.

## Ask AI says it isn't available

Apple's on-device model needs macOS 26, an eligible Mac, Apple Intelligence turned on, and the model downloaded. Or pick another provider in **AI Models**.

## A plugin shows "Plugin error"

The plugin threw an error or ran past its time budget. Other plugins keep working. Run moo with `MOO_DEBUG=1` from a terminal to see the log.

## Logs

```sh
MOO_DEBUG=1 /Applications/Moo.app/Contents/MacOS/moo 2>&1 | tee /tmp/moo.log
```

Still stuck? [Open an issue](https://github.com/moomoi/moo/issues) with the log attached.
