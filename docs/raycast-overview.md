# Raycast, and what Nimble takes from it

Nimble is a Raycast-class launcher for macOS written entirely in Tish. This page summarizes how
Raycast is built, so the rest of the docs can say what Nimble keeps, changes or drops.

## How Raycast is built

**Shell.** A native Swift/AppKit app. A global hotkey toggles a floating, borderless panel. The
root search ranks apps, commands, quicklinks and fallback actions by fuzzy match and frecency (how
often and how recently each item was used). Commands can have their own hotkeys and aliases, and
`raycast://` deeplinks open them from anywhere.

**Extensions.** Written in React and TypeScript and run in a separate Node.js process. A custom
React reconciler turns the component tree into a serializable description and sends it over IPC;
the app renders it with native components:

- `List` (with sections, items, empty view), `Grid`, `Detail` (markdown), `Form` and its fields
- `ActionPanel` / `Action` (the Cmd+K menu)
- `MenuBarExtra` (menu-bar commands)

So Raycast already renders natively. Its memory cost comes from Node and from each extension's
JavaScript heap, not from the UI.

**Manifest.** Each extension's `package.json` lists `commands[]`, each with a `mode` of `view`,
`no-view` or `menu-bar`, plus command `arguments`, `preferences`, a background refresh `interval`,
and `tools[]` for AI.

**API surface** (`@raycast/api`, `@raycast/utils`):

- Toast and HUD, Clipboard, LocalStorage and Cache, `environment`
- `launchCommand`, an OAuth PKCE client, `AI.ask`
- Hooks: `useFetch`, `usePromise`, `useCachedState`, `useSQL`, `useExec`, `useAI`

**AI.** Quick AI, AI Chat, and AI Commands (prompt templates with `{selection}`, `{clipboard}` and
`{argument}` placeholders). Extensions can expose `tools` for function calling, mentioned with `@`
in chat. MCP servers, bring-your-own-key, and local models through Ollama.

**Built-ins.** App launcher, file search (Spotlight metadata), clipboard history, snippets (text
expansion), window management, calculator, emoji, system commands and script commands.

**Store.** A reviewed monorepo; extensions are built from source by Raycast.

## What Nimble keeps, changes and drops

| Raycast | Nimble |
| --- | --- |
| Native AppKit shell | Native AppKit shell, written in Tish (`tish:macos` + `tish:nimble`) |
| Extensions in a Node process | No Node, no JavaScript engine, no webview. Plugins are Tish: bytecode in a capability-free VM (Tier A) or Tish-compiled native modules (Tier B) |
| React reconciler sends UI over IPC | Plugins return plain data (today: list rows and actions); Lattish-style JSX for richer views is planned |
| `package.json` manifest | `nimble.json` (id, tier, entry) plus a `manifest()` the plugin returns at load |
| Frecency ranking | Done: decayed use counts added to the fuzzy score |
| Spotlight file search | Done: in-process `MDQuery`, no extra index |
| Clipboard history | Done: in memory, skips concealed items |
| AI with BYOK and Ollama | Planned: Hypery remote, local Uzu, any OpenAI-compatible local server |
| Reviewed source store | Planned: signed `.nimbleplugin` packages; third-party code runs as Tier A or in a sandboxed helper |

The main bet is that the expensive part of Raycast, a JavaScript runtime per extension, can be
replaced by small Tish VMs and compiled modules while keeping native rendering. The measured cost
so far is 31 MB of memory with two plugins loaded (see [architecture.md](architecture.md)).
