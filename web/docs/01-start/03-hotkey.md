---
title: The hotkey
summary: One key combination opens moo from anywhere. Here's how it's chosen and how to change it.
---
## The default

moo tries these combinations in order and takes the first one that's free:

1. [[⌘ Space]]
2. [[⌥ Space]]
3. [[⌃ ⌥ Space]]
4. [[⌘ ⇧ Space]]

A combination macOS already uses is skipped, because macOS would accept it and then never deliver the keypress. These include Spotlight on [[⌘ Space]], switching input sources on [[⌃ Space]], and Finder search on [[⌘ ⌥ Space]].

> **Tip** To give moo [[⌘ Space]], turn off Spotlight's shortcut in **System Settings › Keyboard › Keyboard Shortcuts › Spotlight**, then set the hotkey below.

## Change it

Open **Settings** ([[⌘ ,]] in the launcher) and choose **Launcher Hotkey**. Press the new keys, then [[Return]]. The change takes effect immediately and is saved as `"launcher"` in [shortcuts.json](/docs/shortcuts-json).

## Swapped modifier keys

If you remapped modifiers in **System Settings › Keyboard › Modifier Keys** (Command and Control swapped, say), moo reads each keyboard's mapping and registers the keys you actually press. Restart moo after changing those mappings.

## It doesn't open

Another app probably owns the combination. Pick a different one in Settings, or set `MOO_HOTKEY` (see [Environment variables](/docs/environment)). Chrome's "Ask Gemini" bar uses [[⌥ Space]] too, so moo opens and immediately loses focus.
