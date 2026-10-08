---
title: Shortcuts
description: Your own keywords for searches, folders, scripts, text and AI prompts.
slug: shortcuts
section: Using moo
order: 10
---
A shortcut is a keyword that does something with the text you type after it. Type `g rust traits` and the `g` shortcut searches Google for "rust traits".

Create one with **Create Shortcut**, manage them in **Shortcuts**, or use the [command line](/docs/cli).

## Kinds

| Kind | Does | Example target |
| --- | --- | --- |
| `url` | Opens the URL, with the text URL-encoded | `https://www.google.com/search?q={query}` |
| `open` | Opens an app, file or folder | `~/Downloads` |
| `command` | Runs a moo or plugin command with text | `moo:clipboard` |
| `shell` | Runs a shell command and shows, copies or discards the output | `curl -s ifconfig.me` |
| `text` | Copies text, or expands it as you type | `Best,\nA` |
| `ai` | Sends the prompt to a model | `Translate to French: {query}` |

## Placeholders

| Placeholder | Becomes |
| --- | --- |
| `{query}` | What you typed after the keyword |
| `{clipboard}` | The clipboard's text |
| `{selection}` | The text selected in the frontmost app |
| `{date}` | Today, like 2026-10-03 |
| `{time}` | Now, like 17:20 |

In `shell` shortcuts the query is single-quoted, so whatever you type is passed as text and never run as a command.

## Hotkeys

Any shortcut can have its own global hotkey that runs it without opening the launcher:

```sh
moo shortcut add ip shell 'curl -s ifconfig.me' --output copy --hotkey ctrl+alt+i
```

## Snippets

A `text` shortcut with `"expand": true` replaces its keyword wherever you type it, in any app:

```sh
moo shortcut add ';sig' text 'Best,\nA' --expand
```

> **Warning** Choose keywords you'd never type by accident. A leading `;` works well.
