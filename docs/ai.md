# AI

Nimble's AI features route through one provider interface. Apple's on-device model is the default
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
`packages/nimble-macos/swift/ai.swift` wraps it in a C ABI (`nimble_ai_availability`,
`_session_new`, `_ask`, `_cancel`, `_prewarm`), compiled to a static library by `build.rs` and linked
into the one binary. Replies stream on a Swift concurrency thread; `src/ai.rs` queues events,
collapses consecutive partial replies, and delivers them on the main queue. Tish sees `aiAvailability`,
`aiSession`, `aiAsk(session, prompt, onEvent)`, `aiCancel`, `aiPrewarm` and `aiEndSession`.

**Older macOS.** Every FoundationModels use is behind `#available`, so the framework is weak-linked
and Nimble still launches on macOS 14; `aiAvailability()` then returns `requires macOS 26`. Other
reasons it reports: `deviceNotEligible`, `appleIntelligenceNotEnabled`, `modelNotReady`. The Swift
runtime comes from `/usr/lib/swift`, which needs a deployment target of 12 or later (Nimble uses 14).

**Limits.** A small model with a context window Apple documents as 4,096 tokens: good for short
answers, rewriting and extraction, weak on long input and broad knowledge. Long conversations fail
with `exceededContextWindowSize` (shown as a message; esc starts over). Apple's safety filter can
refuse (`guardrailViolation`).

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
  Loopback redirects to `127.0.0.1` on any port are allowed. Refresh tokens last 90 days and
  rotate on use. On macOS prefer `ASWebAuthenticationSession`, with the loopback listener as
  fallback. API keys work as an alternative.
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
