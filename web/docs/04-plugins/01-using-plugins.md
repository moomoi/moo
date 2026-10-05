---
title: Using plugins
summary: Plugins add commands to search. They're small, fast and sandboxed.
---
A plugin's commands show up in root search next to everything else. Some take arguments: type the command's keyword and a space, like `slack `, to fill them in.

## Built in

| Plugin | Adds |
| --- | --- |
| Dev Toolbox | Base64, URLs, JWTs, JSON, hashes, colors, timestamps, lorem ipsum (`dev`) |
| GitHub | Your pull requests, reviews, issues, notifications; repository search (`gh`) |
| Package Search | npm, crates.io, Homebrew and PyPI (`npm`, `crate`, `brew`, `pip`) |
| Weather | Now and the next 7 days (`weather paris`) |
| Slack | Send messages, search, open channels (`slack`) |
| Unit Converter | A list of every conversion for what you typed |
| Moo Utils | Generate UUID, Search Emoji, Developer Links |

Browse them all in the [marketplace](/marketplace). Their source is public at [moomoi/plugins](https://github.com/moomoi/plugins). **Install** on a plugin's page opens moo at that plugin's commands, through moo's `moo://` link (`moo://plugins/<id>`).

## Installing more

moo loads every compiled plugin (`.tishc`, `.lib` or `.dylib`) in its plugin folder. To use a folder of your own, set `MOO_PLUGINS` before starting moo:

```sh
export MOO_PLUGINS=~/moo-plugins
```

## Safety

Most plugins are **Tier A**: bytecode that runs in its own VM with no access to files, processes or the network. The only way out is moo's host API, and network access only reaches the hosts listed in the plugin's manifest. Each call has a time budget, so a stuck plugin can't freeze the launcher. If a plugin fails, it shows an error and everything else keeps working.

See [Plugin API](/docs/plugin-api) for the full rules.
