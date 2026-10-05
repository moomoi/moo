# AI

Moo's AI features route through one provider interface. Apple's on-device model is the default
and is built (Quick AI, below). Hypery for remote models, Uzu, and OpenAI-compatible servers on
localhost are designed but not built.

## Built: Quick AI on Apple's on-device model

Apple's `FoundationModels` framework (macOS 26+, Apple Intelligence on) runs a small model on the
device: free, offline, nothing leaves the Mac. Measured on this Mac (macOS 26.6, Apple Silicon):
about 2 s to the first word on a cold start, then 0.3–0.5 s for short answers.

**Use.** Type a question at the root and press tab, or pick the "Ask AI “…”" row that appears
when results leave room, or open the "Ask AI" command. The answer streams into the rows (wrapped
across the full width, up/down to scroll). Type a follow-up and press enter: one session per visit
keeps the conversation's context. Enter on an empty field copies the answer; esc stops a streaming
answer, then leaves.

**How it is wired.** FoundationModels is Swift-only (no Objective-C headers), so
`packages/moo-macos/swift/ai.swift` wraps it in a C ABI (`moo_ai_availability`,
`_session_new`, `_ask`, `_cancel`, `_prewarm`), compiled to a static library by `build.rs` and linked
into the one binary. Replies stream on a Swift concurrency thread; `src/ai.rs` queues events,
collapses consecutive partial replies, and delivers them on the main queue. Tish sees `aiAvailability`,
`aiSession`, `aiAsk(session, prompt, onEvent)`, `aiCancel`, `aiPrewarm` and `aiEndSession`.

**Older macOS.** Every FoundationModels use is behind `#available`, so the framework is weak-linked
and Moo still launches on macOS 14; `aiAvailability()` then returns `requires macOS 26`. Other
reasons it reports: `deviceNotEligible`, `appleIntelligenceNotEnabled`, `modelNotReady`. The Swift
runtime comes from `/usr/lib/swift`, which needs a deployment target of 12 or later (Moo uses 14).

**Tools.** The model can act, not just answer. `aiSession(instructions, toolsJson, onTool)` takes
tools as `[{ name, description, params: [{ name, description, optional?, choices? }] }]` (string
parameters; `choices` limits one to fixed values); Swift turns each into a FoundationModels `Tool`
with a `DynamicGenerationSchema`. When the model calls one, the Swift thread waits on the main queue
while `onTool(name, argsJson)` runs in Tish, and the returned text goes back to the model. Every
call is logged to stderr as `moo: AI tool <name> <args> -> <first line of the reply>`.

Without a tool the model cannot see the Mac, and it will invent an answer ("I couldn't find any
large files") rather than say so. The tools, defined in `aiTools()` in `main.tish`:

| Tool | What it does |
| --- | --- |
| `open` | App name, path or URL; `~` expanded, a missing path retries its last part under home |
| `copyText` | Put text on the clipboard |
| `findFiles` | Spotlight metadata search: name, words in the file's text (`contains`), kind, minimum size, created / modified / opened within, folder, sorted by size, date or name |
| `revealFile` | Select a file in Finder |
| `findApps` | Installed apps by name (the app index) |
| `runningApps` | Apps with a Dock icon, frontmost and hidden marked |
| `clipboardHistory` | Moo's in-memory clipboard history, optionally filtered |
| `systemInfo` | macOS version, model, chip, cores, memory, disk free, battery, uptime |
| `runShortcut` | Run a user shortcut by keyword (listed in the tool description); commands are left to the user |
| `controlMac` | Dark mode, volume and mute, lock screen, sleep displays, screen saver, hide all apps, eject disks. Restart, shut down, sleep, log out, quit all apps and empty Trash are choices too, but the tool refuses them and tells the model to name the command for the user: with no matching choice, the on-device model picked the nearest action and claimed it had restarted |
| `arrangeWindow` | Move the window the user was working in: halves, quarters, thirds, maximize, center, another display, restore |
| `calculate` | The launcher's calculator: arithmetic, percentages, units, currency (ECB rates), number bases, time zones. The instructions tell the model to use it for any maths, since it knows neither today's rates nor the time |
| `define` | The Mac's dictionary: pronunciation, the first four senses with examples, and the origin. The instructions tell the model to answer word questions from it; without that line the on-device model answered "what does petrichor mean" from memory, at length and with invented detail |

`findFiles` is `queryFiles` in Rust (`files::find`, an in-process `MDQuery`). Arguments arrive as
text ("100 MB", "2 days") and are parsed in Tish. "Largest" with no size floor narrows from 1 GB
down until enough files qualify, and "newest" with no date range does the same from the last day,
so the top of the list is right. Only app bundle internals are skipped: build output, caches and
hidden folders count when the question is what takes up space. If no file reaches the model's size
floor, the largest files matching the other filters come back instead.

Tool results that are files, apps or clipboard entries also appear as rows under the answer (at
most four answer lines stay visible above them). ↓ scrolls the answer, then walks the rows; ↵ opens
or copies the selected row, ⌘↵ shows it in Finder, ⌥↵ copies its path, esc deselects. `moo ask`
uses the same tools and prints only the answer.

Checked with `moo ask` on this Mac: "find large files created in the last 2 days" called
`findFiles {minSize: 100 MB, createdWithin: 2 days}` and listed the 8 largest; "which apps are open
right now?", "do I have any photo editing apps installed?", "how much disk space do I have left and
what's my battery at?", "run my stamp shortcut" and "show … in Finder" each called the matching
tool. The instructions include the home folder; with it, "open finder to my documents" called `open`
with `/Users/<you>/Documents` in 3 of 3 runs (`ai::tests::model_calls_a_host_tool`).

**Limits.** A small model with a context window Apple documents as 4,096 tokens: good for short
answers, rewriting and extraction, weak on long input and broad knowledge. Long conversations fail
with `exceededContextWindowSize` (shown as a message; esc starts over). Apple's safety filter can
refuse (`guardrailViolation`); Moo uses the `permissiveContentTransformations` guardrails, Apple's
less strict level for plain-text replies, and puts a refused question back in the field.

## Features

- **Quick AI:** type a question in root search and get a streamed answer in the panel. Built on
  Apple's model (above); Hypery as a remote option is not built.
- **Chat:** a conversation view with history.
- **AI Commands:** saved prompt templates with `{selection}`, `{clipboard}` and `{argument}`
  placeholders, listed in root search like any command.
- **Plugin tools:** plugins declare `tools[]` (JSON-schema functions) in their manifest; chat can
  call them, and the user can `@`-mention a plugin. Every tool call passes a permission gate
  (allow, deny or ask).

## Provider interface

```
chat(messages, { model, tools, signal }) -> stream of { delta | tool_call | done | error }
embed(texts) -> vectors
models() -> [{ id, provider, context }]
```

A router picks a provider per command or chat from settings, with a fallback chain. Custom model
ids look like `custom:<provider>:<model>`.

### Hypery (remote)

- Base URL `https://hypery.ai/v1`, OpenAI-compatible: `chat/completions` (SSE streaming and tools),
  `models`, `embeddings`, `audio/*`, `images/*`.
- **Auth:** OAuth 2.0 with PKCE (S256) through `/api/oauth/authorize` and `/api/oauth/token`.
  The redirect must be registered with the OAuth app. Moo's app allows only
  `https://moo.moi/callback`, which relays the browser back to Moo's loopback listener (see
  [web.md](web.md)). Refresh tokens last 90 days and rotate on use. API keys work as an
  alternative.
- **Key resolution order:** explicit key, then the stored OAuth token, then an environment
  variable. (Skipping the stored-token step caused 401s in Dune.)
- **Token storage:** the Keychain, as one cached blob, with keychain work off the main thread and a
  plaintext presence index so checking for a token never triggers a prompt.
- **Errors** carry a structured `error.code`. The UI branches on it:

| Code | UI |
| --- | --- |
| `INSUFFICIENT_CREDITS` | "Add credits" with a link |
| `SPENDING_LIMIT_EXCEEDED`, `LIMIT_EXCEEDED` | "Limit reached" with a link to settings |
| `RATE_LIMITED` | Retry with backoff, then a message |
| `UNAUTHENTICATED`, `OAUTH_TOKEN_REQUIRED`, `INVALID_KEY` | Refresh the token once, then ask to sign in |
| `INVALID_REQUEST` | Show the message; do not retry |

### Local Uzu

Through `tish-mlx-burn`, which runs Uzu bundles (not GGUF) on Apple Silicon. Inference runs on a
worker thread and streams tokens to the main thread.

### Local OpenAI-compatible

Ollama, llama.cpp server or LM Studio on localhost. This is the cheapest way to support GGUF
models. Local endpoints get a `-` key sentinel so a Hypery key is never sent to them.

## Streaming

Tish has no event loop and `await` blocks the thread, so every stream runs on a worker thread and
posts chunks to the main thread. Details that matter:

- Use `fetch(url, { timeout: 0 })` for streams; a total timeout cuts long answers off.
- Parse SSE at the byte level with correct UTF-8 decoding across chunk boundaries, handling
  `\r\n` and `[DONE]` (reuse claw-code's `sse_parser.tish` and `utf8.tish`).
- Accumulate OpenAI `delta.tool_calls` fragments by index until the call is complete.
- Cancel with an explicit per-request flag checked between chunks, not with `Promise.race`.

## Agent loop

`runAgentTurn` from hellaskills (OpenAI `tool_calls`, `maxTurns` limit), gated by claw-code's
`assertToolAllowed`. AI tools get fixed endpoints so a prompt injection cannot turn them into a
general-purpose fetch.

## Later

MCP client support (HTTP transport first; Tish lacks two-way child-process stdio today) and local
embeddings (the `tish-candle` embedder is still a stub, so use Hypery `/v1/embeddings` meanwhile).
