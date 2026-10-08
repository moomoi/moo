---
title: Command line
description: The moo binary is also a CLI. Script it, or wire it to any hotkey tool.
slug: cli
section: Configuration
order: 20
---
## Install

Link the binary onto your `PATH`:

```sh
ln -s /Applications/Moo.app/Contents/MacOS/moo /usr/local/bin/moo
```

A call to a running moo takes about 5 ms. If moo isn't running, the CLI starts it.

## Commands

```sh
moo                                  # show the launcher
moo run g rust traits                # run a shortcut or command with text
moo run moo:clipboard                # open a command by id
moo search invoice                   # show the launcher with text typed
moo files invoice -n 3 --json        # file paths from the index
moo files 'quarterly report' --contents
moo open-with ~/notes.txt            # apps that open it
moo trash ~/old.txt                  # move to the Trash
moo define serendipity               # the Mac's dictionary
moo web how to make sourdough        # search suggestions
moo apps safari                      # matching applications
moo ask "summarize: $(pbpaste)"      # Ask AI; the answer streams
moo clipboard -n 5                   # clipboard history
moo list commands --json             # shortcuts, commands or hotkeys
moo status
moo config                           # path of shortcuts.json
moo help
```

## Shortcuts and hotkeys

```sh
moo shortcut add g url 'https://www.google.com/search?q={query}'
moo shortcut add proj open .
moo shortcut add ip shell 'curl -s ifconfig.me' --output copy --hotkey ctrl+alt+i
moo shortcut add ';sig' text 'Best,\nA' --expand
moo shortcut rm ip

moo hotkey add cmd+shift+v moo:clipboard
moo hotkey add f5 g weather
moo hotkey add ctrl+alt+s /Applications/Safari.app
moo hotkey rm f5
```

Exit codes are `0` on success and `1` on error, with the reason on stderr.

## With other tools

```sh
# skhd (~/.skhdrc)
cmd + shift - g : moo run g "$(pbpaste)"
alt - space : moo toggle
```

Karabiner-Elements, BetterTouchTool, Hammerspoon, Keyboard Maestro and Shortcuts.app ("Run Shell Script") all work the same way: run `moo run …`.
