---
title: Publishing
summary: Get your plugin into moo and the marketplace with a pull request.
---
Official plugins live in one public repo, [moomoi/plugins](https://github.com/moomoi/plugins). Publishing means getting your plugin merged there. Once it's merged, it ships inside the next moo release and gets its own page in the [marketplace](/marketplace).

## 1. Fork and clone

```sh
gh repo fork moomoi/plugins --clone
cd plugins
```

Or fork on [GitHub](https://github.com/moomoi/plugins/fork) and `git clone` your fork.

## 2. Install

```sh
npm ci
```

This installs the Tish compiler (`@tishlang/tish`) and Lattish from npm, at the versions in `package.json`.

## 3. Add your plugin

Make a folder named after your plugin's id:

```
my-plugin/
  moo.json
  src/plugin.tish
```

```json
{ "id": "my-plugin", "tier": "A", "entry": "src/plugin.tish" }
```

Write `src/plugin.tish` following [Your first plugin](/docs/first-plugin) and the [Plugin API](/docs/plugin-api). The [weather](https://github.com/moomoi/plugins/tree/main/weather) and [github](https://github.com/moomoi/plugins/tree/main/github) plugins are good models for network calls and sign-in.

## 4. Build and try it

```sh
bash build.sh
MOO_PLUGINS="$PWD/dist" open -a Moo
```

Quit moo first so it starts with your folder. Then check every command:

- each one works with an empty query, a normal query and nonsense
- nothing hangs: a call that runs past 250 ms shows "exceeded its 250 ms budget"
- missing setup, like a token, appears as a row that fixes it

## 5. Open a pull request

```sh
git checkout -b add-my-plugin
git add my-plugin
git commit -m "feat: add my-plugin"
git push -u origin add-my-plugin
gh pr create --repo moomoi/plugins
```

In the description, say what the plugin does, list its commands, and say why it needs each host in `permissions.network`. A screenshot of it in moo helps. CI builds every plugin on each push, so the build must pass.

## What gets merged

- **Tish only.** No Rust and no JavaScript.
- **Tier A.** Sandboxed bytecode. Native Tier B plugins are first-party only.
- **Least access.** Every host you call is in `permissions.network`, and nothing else is.
- **Fast.** `list` runs on every keystroke. Start a request, return "Loading…", and call `moo.refresh()` when the answer arrives.
- **One folder.** Your plugin doesn't change anything outside its own folder.

## After it's merged

- It's bundled into the next moo release, and anyone who updates gets it.
- It gets a marketplace page at `moo.moi/marketplace/<id>`, whose Install button opens it in moo.

## Before it's merged

Anyone can try your plugin now: send them the built `dist/my-plugin.tishc` and have them put it in their `MOO_PLUGINS` folder. Tier A plugins are sandboxed, so that's safe.
