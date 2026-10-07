---
title: Keyboard reference
summary: Every key moo listens to, on one page.
---
## In the launcher

| Key | Action |
| --- | --- |
| [[Return]] | Open or run |
| [[Tab]] | Ask AI, or open a command's arguments |
| [[Esc]] | Back, then close |
| [[↑]] [[↓]] [[⌃ P]] [[⌃ N]] | Move the selection |
| [[⌘ K]] | Action panel |
| [[⌘ ,]] | Settings |
| [[⌘ 1]] – [[⌘ 6]] | The bar's pinned items (by default Applications, Files, Actions, Clipboard). Pin any command or app with [[⌘ K]] → Pin to Bar; reorder in Settings → Pinned in Bar |
| [[⌘ L]] | Grid or list view |
| [[⌘ Y]] | Quick Look |
| [[⌘ R]] | Show in Finder |
| [[⌘ O]] | Open with |
| [[⌘ H]] | Hide app (in Running Apps) |
| [[⌘ Return]] | Show in Finder (AI results) |
| [[⌥ Return]] | Copy path |

## In a command's arguments

| Key | Action |
| --- | --- |
| [[Tab]] / [[⇧ Tab]] | Next / previous argument |
| [[Return]] | Pick the choice, then go to the next argument or run |
| [[⌫]] in an empty field | Previous argument |
| [[Esc]] | Back |

## Changing keys

Move any panel action to other keys, or to none, with `"keys"` in [shortcuts.json](/docs/shortcuts-json):

```json
{ "keys": { "actions": "cmd+j", "quicklook": "none" } }
```

The action names are `actions`, `settings`, `category1` to `category6`, `appsview`, `quicklook`, `reveal`, `openwith` and `hideapp`.
