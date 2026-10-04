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
| File search | Done: own live name index, 0.7–3.6 ms over ~600K entries, 18 MB, idle CPU 0; Spotlight while it builds. "Search Files" with reveal (⌘↵) and copy path (⌥↵) |
| Clipboard history | Done: in memory, skips concealed items |
| System commands | Done: Lock Screen, Sleep, Sleep Displays, Screen Saver, Toggle Dark Mode, Toggle Mute, Volume Up / Down ("volume 30" sets it), Eject All Disks, Hide / Quit All Apps, Empty Trash, Restart, Shut Down, Log Out. Destructive ones need a second Return on the same row and query; hotkeys and shortcuts run them at once. Also `nimble system`, `nimble volume`, `nimble mute`, `nimble dark-mode` and the AI's `controlMac` |
| Window management | Done: 21 commands (halves, quarters, thirds, two thirds, Maximize Window, Almost Maximize, Maximize Height, Center Window, Reasonable Size, Next / Previous Display, Restore Window) on the window behind the panel, through Accessibility. Bind any to a hotkey. Also `nimble window <layout>` and the AI's `arrangeWindow` |
| Selected text | Done: `{selection}` in shortcut templates and `nimble selection` read it through Accessibility |
| Quit Applications / switcher | Done: "Running Apps" lists Dock apps by memory use; ↵ switches, ⌘H hides or shows, ⌘⌫ quits, ⌥↵ twice force quits. Also `nimble running [action name]` |
| Calculator | Done: an answer row above the results for arithmetic ("2^10", "15% of 80"), units ("5 km in mi"), currency ("100 usd to eur", ECB rates cached 12 h), number bases ("255 in hex") and time zones ("time in tokyo", "3pm pst to cet"). Return copies the plain answer. Also `nimble calc` and the AI's `calculate` tool |
| Quicklinks, snippets, script commands | Done, in Universal Launcher's simpler form: a keyword bound to a `url`, `open`, `command`, `shell` or `text` target with a `{query}` template, in one hand-editable `shortcuts.json`. "Create Shortcut" builds one in a few keystrokes |
| Snippet expansion | Done: a `text` shortcut with `"expand": true` ("Expand a snippet as you type" in Create Shortcut, or `nimble shortcut add ';sig' text '…' --expand`) replaces its keyword wherever it is typed. Nimble watches typed characters with an AppKit global monitor (no events from password fields) and swaps the keyword through Accessibility after checking it is really before the cursor; no keystrokes are simulated, so apps that do not expose their text fields to Accessibility are left alone |
| Hotkeys for any command | Done: any number of global hotkeys, each bound to a shortcut or command (with optional text), every ANSI key and F1–F20, conflict detection against other bindings and macOS shortcuts, recorder in the panel |
| Deeplinks (`raycast://`) | A CLI instead: the `nimble` binary talks to the running app over a Unix socket in about 5 ms, so skhd, Karabiner, BetterTouchTool or any script can run shortcuts, search files, ask the AI and manage shortcuts |
| AI with BYOK and Ollama | Planned: Hypery remote, local Uzu, any OpenAI-compatible local server |
| Reviewed source store | Planned: signed `.nimbleplugin` packages; third-party code runs as Tier A or in a sandboxed helper |

## Spotlight patterns

The panel follows the macOS 26 (Tahoe) Spotlight wherever it fits a launcher (reference:
[PCMag, "Apple's improved Spotlight feature in macOS is a real game changer"](https://www.pcmag.com/how-to/apples-improved-spotlight-feature-in-macos-is-a-real-game-changer)).
Apps and files are already indexed when the panel first opens, so there is no onboarding screen.

| Spotlight | Nimble |
| --- | --- |
| Opens as a compact search pill with Applications, Files, Actions and Clipboard buttons beside it | Done: a 52 pt bar, field capsule plus four Liquid Glass circles that spring out of the field when it opens |
| Typing expands into results grouped by category, best match first | Done: Calculator (when the text is a sum, conversion or time), Top Hit, then Applications, Files, Actions and Ask AI headings |
| Inline answers for sums, conversions and the time elsewhere | Done: see Calculator above |
| ⌘1–⌘4 (or a button) narrow to Applications, Files, Actions, Clipboard | Done: the field shows the category's symbol and "Search Files" etc.; the same key, ⌫ in an empty field, or esc goes back. Text already typed carries over |
| Applications shows every app as a grid of icons | Done: 7 × 4 icons A–Z with names, arrows move, scrolls by rows; ⌘L switches to a list (remembered) |
| "/" to filter the search | Done: "/" lists the categories; "/files report" opens Files searching "report" |
| ↑ shows previous searches | Done: ↑ in the empty bar lists recent searches (`history.txt`, newest first); a query is recorded when something is opened from it |
| ↓/↑ move, Return opens | Done; headings are skipped and the list scrolls by slots |
| ⌘R (or ⌘ double-click) shows the item in Finder | Done for files, folders and apps |
| Esc clears the query, then closes | Done |
| Actions with quick keys ("ac", "sp") and an "Add quick keys" button | Shortcut keywords are the quick keys, listed under Quick Keys in Actions; "Create Shortcut" adds one |
| Actions with parameter fields that run in the background | Planned, with AI Commands and system actions |
| → cycles categories inside results; Space opens Quick Look | Planned (Space cannot be taken from the focused field; Quick Look will be a key with a modifier) |
| ⌥⌘Space opens a Finder search window | Not planned |
| Calendar, Mail, Messages, Contacts, Music, web results | Planned: definitions, contacts, mail and web suggestions |

The CLI drives the same states for scripts and tests: `nimble search <text>`,
`nimble category applications|files|actions|clipboard|recent [text]` and `nimble history [--clear]`.

The main bet is that the expensive part of Raycast, a JavaScript runtime per extension, can be
replaced by small Tish VMs and compiled modules while keeping native rendering. The measured cost
so far is 31 MB of memory with two plugins loaded (see [architecture.md](architecture.md)).
