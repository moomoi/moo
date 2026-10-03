# Plugin API

A Nimble plugin is Tish code that adds commands to the root search. This page documents contract
v0, which both example plugins implement, and then what is planned next.

## Layout

```
plugins/<name>/
  nimble.json        { "id": "...", "tier": "A" | "B", "entry": "src/plugin.tish" }
  src/plugin.tish
```

`plugins/build.sh` builds every plugin into `plugins/dist/` according to its tier:

| Tier | Build | Output | Runs as |
| --- | --- | --- | --- |
| A | `tish build <entry> --target bytecode` | `<id>.tishc` | bytecode in a `tish_vm` with no capabilities |
| B | `tish build <entry> --target native --crate-type cdylib` | `<id>.lib` | native module loaded in-process via `tish:ffi` |

The shell loads every `.tishc`, `.lib` and `.dylib` in its plugin folder: `NIMBLE_PLUGINS` if set,
otherwise `Contents/Resources/plugins` inside `Nimble.app`, otherwise `../plugins/dist` when run
from `app/`.

## Contract v0

A plugin provides three functions:

- `manifest()` returns `{ id, title, commands }`. Each command is
  `{ name, title, subtitle, mode }`, where `mode` is `"no-view"` or `"list"`.
- `run(command)` runs a `no-view` command and returns one action (or `null`).
- `list(command, query)` returns the rows of a `list` command for the current query. It is called
  again on every keystroke. Each row is `{ title, subtitle, action }`.

Commands appear in root search under their `title`, ranked alongside apps. Choosing a `list`
command opens it in the panel; Esc goes back to root.

### Actions

An action is a plain object. The shell applies the fields it finds, in this order:

| Field | Effect |
| --- | --- |
| `copy: string` | Put the text on the clipboard |
| `hud: string` | Show a message in the panel's status line |
| `open: string` | Open a URL (`scheme://...`) or a file path, then return to root |
| `close: true` | Hide Nimble and return to root |

### Tier B: export the functions

```tish
export fn manifest() {
  return { id: "utils", title: "Nimble Utils", commands: [
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
or `tish:ffi` fails (tests in `packages/nimble-macos/src/vmplug.rs` check each one). `Math`,
strings, arrays, objects and closures work (the converter plugin uses all of them). A Tier A plugin can compute and return data,
nothing else. I/O will come only through host APIs gated by declared permissions (below).

### Errors

A throw from `run` or `list` shows "Plugin error: ..." in the status line. A plugin that fails to
load is logged and skipped; the rest still load.

## Planned

**Manifest in `nimble.json`.** Move commands out of `manifest()` so the store can read them without
running code, and add the Raycast fields: command `arguments`, `hotkey`, `interval`, `preferences`,
`permissions` and AI `tools[]` (JSON-schema function definitions).

**Host API module `tish:nimble` for plugins.** Each host function is one registered entry with a
schema, a required permission and a frozen name; plugin typings are generated from that registry.
Every call checks the plugin's granted permissions, carried as a `tish-biscuit` capability token
minted per plugin and attenuated per command. First entries: `fetch` (on a host thread, with a
host allowlist from `permissions.network`), `clipboard`, `storage`, `toast`, `ai`.

**Views.** Lattish-style JSX components mirroring Raycast's (`List`, `List.Item`, `Detail`, `Form`,
`ActionPanel`), sent to the shell as plain data. The shell owns row reuse, so long lists stay cheap.

**Threads.** One worker thread per Tier A VM; calls are messages to that thread and results are
posted to the main thread.

**Tier C.** Untrusted native plugins in a sandboxed `nimble-plughost` helper over a Unix-socket
RPC carrying the same contract. Only that helper gets `disable-library-validation`; the main app
keeps library validation on, so only cdylibs signed with Nimble's Team ID load in-process.

**Packaging.** A `.nimbleplugin` archive (manifest, chunk or dylib, assets) signed with ed25519,
unpacked with traversal guards and size limits into a per-version folder.
