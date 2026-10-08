---
title: shortcuts.json
description: One hand-editable file holds your hotkeys, shortcuts, aliases and AI setup.
slug: shortcuts-json
section: Configuration
order: 19
---
moo keeps its settings in `~/.config/moo/shortcuts.json`. Settings, the command line and the file itself all edit the same data, and moo picks up changes to the file as soon as it's saved. **Edit shortcuts.json** in the launcher opens it, and `moo config` prints its path.

## Example

```json
{
  "launcher": "cmd+space",
  "shortcuts": [
    { "keyword": "g", "name": "Google", "kind": "url", "target": "https://www.google.com/search?q={query}" },
    { "keyword": "dl", "name": "Downloads", "kind": "open", "target": "~/Downloads", "hotkey": "ctrl+alt+d" }
  ],
  "hotkeys": [{ "keys": "cmd+shift+v", "run": "moo:clipboard" }],
  "aliases": { "moo:clipboard": "cb" },
  "disabled": ["moo:system:empty-trash"],
  "keys": { "actions": "cmd+j" },
  "search": "https://duckduckgo.com/?q={query}",
  "ai": { "model": "hypery:gpt-5-mini" }
}
```

## Keys

| Key | |
| --- | --- |
| `launcher` | The [hotkey](/docs/hotkey) that opens moo |
| `shortcuts` | Your [shortcuts](/docs/shortcuts) |
| `hotkeys` | Global hotkeys that run a command, shortcut or app |
| `aliases` | Short names that bring a command to the top |
| `disabled` | Commands and apps to leave out of search |
| `keys` | Launcher [keys](/docs/keyboard) that differ from the defaults |
| `search` | The web search engine |
| `ai` | The default model and extra [providers](/docs/models) |

## Shortcut fields

| Field | |
| --- | --- |
| `keyword` | What you type |
| `name` | What the row says |
| `kind` | `url`, `open`, `command`, `shell`, `text` or `ai` |
| `target` | The URL, path, command, script, text or prompt |
| `hotkey` | *Optional.* Runs it from anywhere |
| `input` | `command` only: the search text to start with (default `{query}`) |
| `output` | `shell` only: `show`, `copy` or `none` |
| `model` | `ai` only: `provider:model` |
| `expand` | `text` only: expand as you type |

## Naming things

`hotkeys`, `aliases` and `disabled` name what they run:

- a command id like `moo:clipboard` or `moo:window:left-half`
- a shortcut, as `shortcut:<keyword>`
- an app, by its path

`moo list commands` prints every command id.
