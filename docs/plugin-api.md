# Plugin API

A Moo plugin is Tish code that adds commands to the root search. This page documents contract
v0, which both example plugins implement, and then what is planned next.

## Layout

```
plugins/<name>/
  moo.json        { "id": "...", "tier": "A" | "B", "entry": "src/plugin.tish" }
  src/plugin.tish
```

`plugins/build.sh` builds every plugin into `plugins/dist/` according to its tier:

| Tier | Build | Output | Runs as |
| --- | --- | --- | --- |
| A | `tish build <entry> --target bytecode` | `<id>.tishc` | bytecode in a `tish_vm` with no capabilities |
| B | `tish build <entry> --target native --crate-type cdylib` | `<id>.lib` | native module loaded in-process via `tish:ffi` |

The shell loads every `.tishc`, `.lib` and `.dylib` in its plugin folder: `MOO_PLUGINS` if set,
otherwise `Contents/Resources/plugins` inside `Moo.app`, otherwise `../plugins/dist` when run
from `app/`.

## Contract v0

A plugin provides these functions:

- `manifest()` returns `{ id, title, icon, permissions, commands }`. Each command is
  `{ name, title, subtitle, mode, icon, keyword, arguments }`, where `mode` is `"no-view"`,
  `"list"` or `"view"`; everything after `mode` is optional.
- `run(command, args)` runs a `no-view` command and returns one action (or `null`).
- `list(command, query, args)` returns the rows of a `list` command for the current query. It is
  called again on every keystroke. Each row is `{ title, subtitle, icon, action }`.
- `suggest(command, argument, text, args)` (optional) returns the choices for a `"dropdown"`
  argument as you type: `[{ title, subtitle, value, icon }]`. A suggestion with an `action`
  instead of a `value` is a help row (Create the app…); a `"text"` argument shows those while its
  field is empty.
- `blocker(command)` (optional) returns what the command needs first, or `null`:
  `{ title, subtitle, icon, action }`, say `{ title: "Sign In to Slack", action: { run: "signin" } }`.
- `visible(command)` (optional) returns `false` to leave a command out of search for now (Sign Out
  while signed out).

`args` is `{ <argument name>: value }` (empty for a command without arguments).

Commands appear in root search under their `title`, ranked alongside apps; a plugin command also
matches its `keyword` and its plugin's title. Choosing a `list` command opens it in the panel; Esc
goes back to root.

An `icon` (on the plugin, a command or a row) is `"symbol:<SF Symbol name>"`, an `https://` image
(downloaded once and cached; the command's icon shows until it arrives) or an AppKit image name.

### Arguments

A command can take inline arguments, filled in the search field the way Raycast does:

```tish
{ name: "send", title: "Send Slack Message", mode: "no-view", keyword: "slack", arguments: [
  { name: "to", placeholder: "To", type: "dropdown", required: true },
  { name: "message", placeholder: "Message", type: "text", required: true }
] }
```

Tab on the command, Return on it, or its `keyword` followed by a space (`slack ` in root search)
opens its arguments. The header shows the command and the values filled in so far, and the field
edits one argument at a time, its `placeholder` as the hint:

| Key | In argument mode |
| --- | --- |
| Tab / ⇧Tab | Next / previous argument (Tab on a dropdown picks the selected choice) |
| Return | On a choice: pick it, then go to the next empty required argument or run. On the command row: run |
| ⌫ in an empty field | Previous argument; in the first one, back to where you came from |
| Esc | Back |

A `"dropdown"` argument lists `suggest(...)`'s choices above the command row; picking one passes its
`value` (the `title` shows in the field). A required dropdown needs a pick; a required text
argument needs some text. Return with one missing goes to it and says so. Then the command runs
like any other with `args` filled in: `run` for `no-view`, `list` for `list`, `open` for `view`.

### Actions

An action is a plain object. The shell applies the fields it finds, in this order:

| Field | Effect |
| --- | --- |
| `copy: string` | Put the text on the clipboard |
| `hud: string` | Show a message in the panel's status line |
| `open: string` | Open a URL (`scheme://...`) or a file path, then return to root |
| `stay: true` | With `open`: stay where you are (the user comes back to paste something) |
| `close: true` | Hide Moo and return to root |
| `run: string` | Open another command: one of the plugin's own by `name`, or any by id (`moo list`) |

### Blockers

Never tell the user to go and run something; offer it. When a command can't work yet (not signed
in, no client ID), `blocker(command)` says what it needs, and the shell makes that the thing to do:

- Inside a `list` command or a command's arguments, the blocker is the only row, selected, so
  Return does it. Once it's resolved, `moo.refresh()` brings the real rows back.
- A `no-view` or `view` command does the blocker's action straight away.
- Commands that make no sense right now are hidden with `visible(command)`.

A blocker's `action` is usually `{ run: "<command>" }`. Chain the steps so each one leads to the
next: Slack's Sign In, with no client ID yet, returns `{ run: "client" }`, whose field offers
"Create Moo's Slack App"; saving the ID starts the sign-in.

The shell does the same for its own blockers. An Ask AI error that needs sign-in, credit or an API
key shows the fix as the selected row (Sign In to Hypery, Add Credits, Add API Key, Choose Another
Model), and after signing in or saving the key the question is asked again.

### Tier B: export the functions

```tish
export fn manifest() {
  return { id: "utils", title: "Moo Utils", commands: [
    { name: "uuid", title: "Generate UUID", subtitle: "Copy a random v4 UUID", mode: "no-view" }
  ] }
}

export fn run(command) {
  return { copy: "...", hud: "Copied" }
}

export fn list(command, query) {
  return []
}
```

### Tier A: call `register`

A bytecode chunk has no exports, so the host injects a `register` global and the plugin hands it
the same three functions. Calling `register` is required; a chunk that never calls it fails to
load with "plugin never called register()".

```tish
let runs = 0

register({
  manifest: () => ({ id: "convert", title: "Unit Converter", commands: [
    { name: "convert", title: "Convert Units", subtitle: "10 km, 72 f", mode: "list" }
  ] }),
  run: (command) => ({ hud: "ran " + command }),
  list: (command, query) => {
    runs = runs + 1
    return [{ title: "10000 m", subtitle: "10 km · length", action: { copy: "10000" } }]
  }
})
```

Module state (`runs` above) lives as long as the VM, so it persists across calls.

### What a Tier A plugin can and cannot do

The VM is created with an empty capability set. Importing `tish:fs`, `tish:process`, `tish:http`
or `tish:ffi` fails (tests in `packages/moo-macos/src/vmplug.rs` check each one). `Math`,
strings, arrays, objects, `JSON` and closures work. Its only way out is the `moo` global below,
which is generic: nothing in the host knows about any particular plugin.

### The `moo` host object (Tier A)

| Call | Does |
| --- | --- |
| `moo.fetch({ method, url, headers, body, form }, cb)` | An HTTPS request on a worker thread; `cb({ status, body, error })`. `form` is sent URL-encoded |
| `moo.store.get(key)` / `set(key, text)` / `remove(key)` | Plain text per plugin, in `~/Library/Application Support/Moo/plugins/<id>.json` (`MOO_PLUGIN_DATA` overrides the folder) |
| `moo.secret.get(key)` / `set(key, text)` / `remove(key)` | The login Keychain, account `plugin.<id>.<key>` |
| `moo.signIn({ authorize, token, clientId, scope, params }, cb)` | Browser OAuth with PKCE through the `https://moo.moi/callback` relay; `cb({ ok, accessToken, refreshToken, expiresAt, error })`. `params` adds authorize parameters |
| `moo.cancelSignIn()` | Stops a pending sign-in |
| `moo.refresh()` | Asks the shell to call `list` / `suggest` again (data arrived) |
| `moo.notify(text)` | A message in the status line (a send finished, a sign-in failed) |
| `moo.log(text)` | One line in Moo's log |

`fetch` and the sign-in URLs may only reach `https://` hosts named in the manifest's
`permissions.network` (a host covers its subdomains): `permissions: { network: ["slack.com"] }`.
Anything else fails with an error naming the missing permission. Callbacks run on the main thread
under the same call budget as the exports.

`run`, `list` and `suggest` are synchronous, so a plugin starts a request, returns what it has
(say "Searching…"), and calls `moo.refresh()` or `moo.notify(text)` when the answer comes.

### The Slack plugin

`plugins/slack` is a Tier A plugin written entirely in Tish against the API above: Send Slack
Message (`slack` then Space: To, then Message), Search Slack Messages, Open Slack Channel, Refresh
Slack Channels, Sign In, Sign Out and Set Slack Client ID.

It signs in with Slack's user-token OAuth (`oauth/v2_user/authorize` and `oauth.v2.user.access`)
with PKCE, so no client secret ships with Moo. It needs a Slack app, registered once:

Moo walks through it: any Slack command leads to Sign In to Slack, which asks for the Client ID
and offers "Create Moo's Slack App" (api.slack.com with `plugins/slack/slack-app.json` filled in).

1. Create the app in the workspace you want.
2. Under OAuth & Permissions, turn on PKCE. Slack can't turn it off again without support, and it
   marks the app as a public client, which is what a desktop app is.
3. To use it in workspaces other than the one it was created in, turn on public distribution
   under Manage Distribution.
4. Copy the Client ID from Basic Information, paste it into Moo and press Return: the browser
   sign-in starts.

The user token is kept in the Keychain; channels and people are cached in the plugin's store for
ten minutes (Refresh Slack Channels reloads them).

### Time budget

Tier A code runs with the JIT off and under a deadline: 1000 ms for the top level (setup plus
`register`), 250 ms for each call to an export. Past the budget the call aborts and throws
`<id>: list() exceeded its 250 ms budget`. Module state stays as the aborted call left it, and the
next call runs normally. `list` runs on every keystroke, so keep it well under the budget.

### Errors

A throw from `run` or `list`, including a blown budget, shows "Plugin error: ..." in the status
line. A plugin that fails to load (including a top level that runs past its budget) is logged and
skipped; the rest still load.

## Planned

**Manifest in `moo.json`.** Move commands out of `manifest()` so the store can read them without
running code, and add the remaining Raycast fields: `hotkey`, `interval`, `preferences` and AI
`tools[]` (JSON-schema function definitions).

**Host API registry.** Each `moo` function as one registered entry with a schema, a required
permission and a frozen name; plugin typings are generated from that registry. Every call checks
the plugin's granted permissions, carried as a `tish-biscuit` capability token minted per plugin
and attenuated per command. Still to add: `clipboard`, `ai`.

**Views.** Lattish-style JSX components mirroring Raycast's (`List`, `List.Item`, `Detail`, `Form`,
`ActionPanel`), sent to the shell as plain data. The shell owns row reuse, so long lists stay cheap.

**Threads.** One worker thread per Tier A VM; calls are messages to that thread and results are
posted to the main thread.

**Tier C.** Untrusted native plugins in a sandboxed `moo-plughost` helper over a Unix-socket
RPC carrying the same contract. Only that helper gets `disable-library-validation`; the main app
keeps library validation on, so only cdylibs signed with Moo's Team ID load in-process.

**Packaging.** A `.mooplugin` archive (manifest, chunk or dylib, assets) signed with ed25519,
unpacked with traversal guards and size limits into a per-version folder.
