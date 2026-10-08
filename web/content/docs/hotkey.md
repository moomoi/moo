---
title: The hotkey
description: One key combination opens moo from anywhere. Here's how it's chosen and how to change it.
slug: hotkey
section: Getting started
order: 3
---
## The default

moo tries these combinations in order and takes the first one that's free:

1. <kbd>⌘</kbd> <kbd>Space</kbd>
2. <kbd>⌥</kbd> <kbd>Space</kbd>
3. <kbd>⌃</kbd> <kbd>⌥</kbd> <kbd>Space</kbd>
4. <kbd>⌘</kbd> <kbd>⇧</kbd> <kbd>Space</kbd>

A combination macOS already uses is skipped, because macOS would accept it and then never deliver the keypress. These include Spotlight on <kbd>⌘</kbd> <kbd>Space</kbd>, switching input sources on <kbd>⌃</kbd> <kbd>Space</kbd>, and Finder search on <kbd>⌘</kbd> <kbd>⌥</kbd> <kbd>Space</kbd>.

> **Tip** To give moo <kbd>⌘</kbd> <kbd>Space</kbd>, turn off Spotlight's shortcut in **System Settings › Keyboard › Keyboard Shortcuts › Spotlight**, then set the hotkey below.

## Change it

Open **Settings** (<kbd>⌘</kbd> <kbd>,</kbd> in the launcher) and choose **Launcher Hotkey**. Press the new keys, then <kbd>Return</kbd>. The change takes effect immediately and is saved as `"launcher"` in [shortcuts.json](/docs/shortcuts-json).

## Swapped modifier keys

If you swapped modifiers in **System Settings › Keyboard › Modifier Keys** (Command and Control, say), moo uses the keys macOS sees, like every other app. Set the hotkey by pressing it in Settings, and what you press is what you get.

## It doesn't open

Another app probably owns the combination. Pick a different one in Settings, or set `MOO_HOTKEY` (see [Environment variables](/docs/environment)). Chrome's "Ask Gemini" bar uses <kbd>⌥</kbd> <kbd>Space</kbd> too, so moo opens and immediately loses focus.
