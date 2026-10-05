---
title: Publishing
summary: Get your plugin into moo and the marketplace with a pull request.
---
Every official plugin lives in [moomoi/plugins](https://github.com/moomoi/plugins). A plugin merged there ships inside the next moo release and gets a page in the [marketplace](/marketplace).

## Submit a plugin

1. Fork [moomoi/plugins](https://github.com/moomoi/plugins) and add a folder with `moo.json` and `src/plugin.tish`.
2. Build everything with `bash build.sh` and try it in moo with `MOO_PLUGINS=$PWD/dist`.
3. Open a pull request. CI builds every plugin on each push.

## What gets merged

- **Tish only.** Plugin source is Tish, with no Rust and no JavaScript.
- **Tier A.** Sandboxed bytecode. Native Tier B plugins are first-party only.
- **Declared hosts.** List every host you call in `permissions.network`, and nothing more.
- **Fast.** Stay well under the 250 ms budget per call; `list` runs on every keystroke.
- **Offer, don't instruct.** Missing setup, like a token, is a `blocker()` row that does it.

## Sharing before it's merged

Send people the built `.tishc` and have them put it in their `MOO_PLUGINS` folder. Tier A plugins are sandboxed, so that's safe to try.

## What's coming

- **Install without a release:** signed `.mooplugin` packages, installed from the marketplace's Install button.
- **Manifest in `moo.json`**, so the marketplace can read commands, hotkeys and preferences without running code.
- **Views:** list, detail and form components rendered natively.
