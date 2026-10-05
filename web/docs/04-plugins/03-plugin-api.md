---
title: Plugin API
summary: The full plugin contract, from the manifest to actions, arguments, blockers and the host object.
---
## Tiers

| Tier | Build | Runs as |
| --- | --- | --- |
| A | `tish build <entry> --target bytecode` → `<id>.tishc` | Bytecode in a VM with no capabilities |
| B | `tish build <entry> --target native --crate-type cdylib` → `<id>.lib` | A native module loaded in-process |

Tier A plugins call `register({ ... })` with their functions. Tier B plugins `export` them. Tier B plugins run with full access, so only first-party plugins use it.

## Functions

| Function | Returns |
| --- | --- |
| `manifest()` | `{ id, title, icon, permissions, commands }` |
| `run(command, args)` | One action for a `no-view` command, or `null` |
| `list(command, query, args)` | The rows of a `list` command: `[{ title, subtitle, icon, action }]` |
| `suggest(command, argument, text, args)` | *Optional.* Choices for a `dropdown` argument |
| `blocker(command)` | *Optional.* What the command needs first, or `null` |
| `visible(command)` | *Optional.* `false` hides the command for now |

## Commands

```tish
{ name: "send", title: "Send Slack Message", mode: "no-view", keyword: "slack", arguments: [
  { name: "to", placeholder: "To", type: "dropdown", required: true },
  { name: "message", placeholder: "Message", type: "text", required: true }
] }
```

`mode` is `"no-view"`, `"list"` or `"view"`. A command matches its `title`, its `keyword` and its plugin's title.

An `icon` is `"symbol:<SF Symbol name>"`, an `https://` image (downloaded once and cached), or an AppKit image name.

## Arguments

[[Tab]] or [[Return]] on a command, or its keyword followed by a space, opens its arguments. The field edits one argument at a time. A `dropdown` shows `suggest(...)`'s choices, and picking one passes its `value`. When every required argument is filled, the command runs with `args` set to `{ <name>: value }`.

## Actions

An action is a plain object. moo applies the fields it finds, in this order:

| Field | Effect |
| --- | --- |
| `copy: string` | Put text on the clipboard |
| `hud: string` | Show a message in the status line |
| `open: string` | Open a URL or a file path |
| `stay: true` | With `open`, stay where you are |
| `close: true` | Hide moo |
| `run: string` | Open another command, by `name` or by id |

## Blockers

Never tell the user to go and run something. Offer it instead. When a command can't work yet (not signed in, say), `blocker(command)` returns a row like `{ title: "Sign In to Slack", action: { run: "signin" } }`. moo makes it the selected row, so [[Return]] fixes it. Call `moo.refresh()` once it's resolved.

## The moo host object

Tier A plugins reach the outside world only through `moo`:

| Call | Does |
| --- | --- |
| `moo.fetch({ method, url, headers, body, form }, cb)` | An HTTPS request; `cb({ status, body, error })` |
| `moo.store.get(key)` / `set` / `remove` | Plain text, saved per plugin |
| `moo.secret.get(key)` / `set` / `remove` | The login Keychain |
| `moo.signIn({ authorize, token, clientId, scope, params }, cb)` | Browser OAuth with PKCE |
| `moo.cancelSignIn()` | Stop a pending sign-in |
| `moo.refresh()` | Call `list` / `suggest` again because data arrived |
| `moo.notify(text)` | A message in the status line |
| `moo.log(text)` | A line in moo's log |

`fetch` and sign-in only reach hosts listed in the manifest, and a host covers its subdomains:

```tish
permissions: { network: ["slack.com"] }
```

`run`, `list` and `suggest` are synchronous. Start a request, return what you have ("Searching…"), and call `moo.refresh()` when the answer arrives.

## Budgets

Tier A code gets **1000 ms** for its top level and **250 ms** for each call. Past that, the call aborts with an error and the next call runs normally. `list` runs on every keystroke, so keep it well under the limit.

## Errors

A throw from `run` or `list` shows "Plugin error: …" in the status line. A plugin that fails to load is logged and skipped, and the other plugins still load.
