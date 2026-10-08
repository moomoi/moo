---
title: Environment variables
description: Override where moo keeps things and how it starts.
slug: environment
section: Configuration
order: 21
---
| Variable | Effect |
| --- | --- |
| `MOO_HOTKEY` | Hotkey to try first, like `ctrl+alt+space`. Overrides Settings |
| `MOO_CONFIG` | The shortcuts file (default `~/.config/moo/shortcuts.json`) |
| `MOO_PLUGINS` | The plugin folder |
| `MOO_PLUGIN_DATA` | Where plugins' stored data goes |
| `MOO_FRECENCY` | Ranking history (default `~/Library/Application Support/Moo/frecency.tsv`) |
| `MOO_SOCKET` | The CLI socket (default `~/Library/Application Support/Moo/moo.sock`) |
| `MOO_START_HIDDEN` | `1` starts without showing the launcher |
| `MOO_DEBUG` | Timestamped logs on stderr |
| `MOO_SYSTEM_DRY_RUN` | System commands only report what they would do |

> **Note** moo inherits the environment of whatever started it. Set variables in the shell before running `moo` or `open -a Moo`, not on a single command.
