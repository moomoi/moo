---
title: Keyboard reference
description: Every key moo listens to, on one page.
slug: keyboard
section: Using moo
order: 11
---
## In the launcher

| Key | Action |
| --- | --- |
| <kbd>Return</kbd> | Open or run |
| <kbd>Tab</kbd> | Ask AI, or open a command's arguments |
| <kbd>Esc</kbd> | Back, then close |
| <kbd>↑</kbd> <kbd>↓</kbd> <kbd>⌃</kbd> <kbd>P</kbd> <kbd>⌃</kbd> <kbd>N</kbd> | Move the selection |
| <kbd>⌘</kbd> <kbd>K</kbd> | Action panel |
| <kbd>⌘</kbd> <kbd>,</kbd> | Settings |
| <kbd>⌘</kbd> <kbd>1</kbd> – <kbd>⌘</kbd> <kbd>6</kbd> | The bar's pinned items (by default Applications, Files, Actions, Clipboard). Pin any command or app with <kbd>⌘</kbd> <kbd>K</kbd> → Pin to Bar; reorder in Settings → Pinned in Bar |
| <kbd>⌘</kbd> <kbd>L</kbd> | Grid or list view |
| <kbd>⌘</kbd> <kbd>Y</kbd> | Quick Look |
| <kbd>⌘</kbd> <kbd>R</kbd> | Show in Finder |
| <kbd>⌘</kbd> <kbd>O</kbd> | Open with |
| <kbd>⌘</kbd> <kbd>H</kbd> | Hide app (in Running Apps) |
| <kbd>⌘</kbd> <kbd>Return</kbd> | Show in Finder (AI results) |
| <kbd>⌥</kbd> <kbd>Return</kbd> | Copy path |

## In a command's arguments

| Key | Action |
| --- | --- |
| <kbd>Tab</kbd> / <kbd>⇧</kbd> <kbd>Tab</kbd> | Next / previous argument |
| <kbd>Return</kbd> | Pick the choice, then go to the next argument or run |
| <kbd>⌫</kbd> in an empty field | Previous argument |
| <kbd>Esc</kbd> | Back |

## Changing keys

Move any panel action to other keys, or to none, with `"keys"` in [shortcuts.json](/docs/shortcuts-json):

```json
{ "keys": { "actions": "cmd+j", "quicklook": "none" } }
```

The action names are `actions`, `settings`, `category1` to `category6`, `appsview`, `quicklook`, `reveal`, `openwith` and `hideapp`.
