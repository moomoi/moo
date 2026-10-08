---
title: Your first plugin
description: A working list plugin in about twenty lines of Tish.
slug: first-plugin
section: Plugins
order: 15
---
Plugins are written in [Tish](https://github.com/tishlang/tish), a small JavaScript-like language that compiles to bytecode or native code. The quickest start is to clone [moomoi/plugins](https://github.com/moomoi/plugins), which has every official plugin to learn from and a `build.sh` that builds them all.

## Layout

```
my-plugin/
  moo.json
  src/plugin.tish
```

`moo.json` names the plugin, its tier and its entry file:

```json
{ "id": "shout", "tier": "A", "entry": "src/plugin.tish" }
```

## The code

A Tier A plugin hands its functions to `register`. This one adds a **Shout** command whose rows are your text in capitals:

```tish
register({
  manifest: () => ({
    id: "shout",
    title: "Shout",
    commands: [
      { name: "shout", title: "Shout", subtitle: "Say it louder", mode: "list", keyword: "shout" }
    ]
  }),
  run: (command) => null,
  list: (command, query) => {
    if (query === "") {
      return [{ title: "Type something…" }]
    }
    let loud = query.toUpperCase() + "!"
    return [{ title: loud, subtitle: "Return to copy", action: { copy: loud, hud: "Copied" } }]
  }
})
```

`list` runs on every keystroke and returns rows. Each row's `action` is what <kbd>Return</kbd> does.

## Build and load

The Tish compiler is on npm:

```sh
npm install -g @tishlang/tish
tish build src/plugin.tish --target bytecode -o ~/moo-plugins/shout.tishc
MOO_PLUGINS=~/moo-plugins open -a Moo
```

Type `shout hello` and you get **HELLO!**

## Next

- [Plugin API](/docs/plugin-api): commands, arguments, actions and the host object
- [Slack plugin](/docs/slack): a complete plugin with sign-in and network calls
