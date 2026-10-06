# Plugin runtime: `.tishc` today, WASM/WASI as an option

Internal notes on how Tier A plugins run, why bytecode (`.tishc`) works for now, what WebAssembly
would change, and a staged plan for moving to it. The measurements were taken on 2026-10-05 on
the main dev Mac (Apple silicon), with the tish 3.15 compiler (today Moo pins `@tishlang/tish`
in `package.json`).

**Short version:** `.tishc` is the right runtime for Tish-only plugins that return rows and make a
few HTTP calls. It's tiny, fast, and has no translation layer between the shell and the plugin.
Its real weaknesses are that the sandbox is homegrown and in-process, and that the bytecode
format isn't stable across Tish versions. Neither matters much while every plugin is first-party
and built from source inside each release. Both start to matter when third parties ship prebuilt
plugins. The recommended path keeps `.tishc` as the plugin format and moves the VM running it
inside a WebAssembly sandbox (Phase 2 below). That gets most of WASM's isolation without
shipping a 3 MB VM per plugin.

No record exists of why `.tishc` was chosen over WASM originally. The rationale below is
reconstructed from the code.

## How plugins run today

| Tier | Built with | File | Runs as |
| --- | --- | --- | --- |
| A | `tish build --target bytecode` | `<id>.tishc` | serialized bytecode in a `tish_vm` inside the Moo process |
| B | `tish build --target native --crate-type cdylib` | `<id>.lib` | native code loaded in-process through `tish:ffi`; first-party only, signed with Moo's Team ID |

The Tier A pipeline:

1. `index.rs` lists `*.tishc` in the plugin folder (`MOO_PLUGINS`, `Contents/Resources/plugins`,
   or `plugins/dist`).
2. `main.tish` `loadPlugin` calls `loadBytecodePlugin(path)` (`vmplug.rs`). This:
   - deserializes the chunk
   - creates a `Vm` with **no capabilities** (no fs, process, http, ffi, timers) and the JIT off
   - injects two globals: `register` and `moo` (`pluginhost.rs`)
   - runs the top level under a 1000 ms deadline
3. The plugin calls `register({ manifest, run, list, suggest, blocker, visible, open })`. The
   closures it passes are ordinary Tish `Value`s, so the shell calls them like any function, each
   under a 250 ms deadline (`CALL_BUDGET_MS`).
4. The plugin's only way out is the `moo` object:
   - `fetch` (https only, to hosts in `permissions.network`)
   - `store` (a JSON file per plugin)
   - `secret` (the Keychain)
   - `signIn` (OAuth with PKCE through the moo.moi relay)
   - `refresh`, `notify`, `log`

   Fetch callbacks come back on the main thread under the same budget.

Everything runs **on the main thread, in Moo's own process**. Values cross between the shell and
the plugin as shared Tish values (`tishlang_core::Value`), with no copying or serialization.

## Why `.tishc` works well

- **No translation layer.** The shell, the host API and the plugins are all Tish. Rows, actions,
  `@moo/ui` view trees and callbacks are plain values. `moo.fetch(req, cb)` just stores `cb` and
  calls it later. With WASM, each of these needs a calling convention defined in bytes.
- **Sandboxing by omission.** The VM starts with an empty capability set. `import "tish:fs"`
  simply fails (tests in `vmplug.rs` check each capability). The whole attack surface is the
  `moo` object, 429 lines in `pluginhost.rs`.
- **Cheap budgets.** Deadlines are a thread-local the VM polls. Turning the JIT off makes
  polling reliable, since a compiled loop would never check the deadline.
- **Small and fast.** Today's plugins are 5–45 KB and load in 0.1–0.4 ms each (see below). No
  second runtime ships in the app.
- **One toolchain.** Authors write Tish and run one `tish build` command. The public plugins repo
  builds everything with a single script and the pinned compiler.
- **Fits the Tish-only policy.** WASM's biggest advantage, any source language, is a non-goal.

## Where `.tishc` is weak

1. **The sandbox is in-process.** The plugin VM shares Moo's address space. A memory-safety bug
   in `tish_vm` (Rust, though any `unsafe` code, or the JIT's code generation if it were
   ever left on, would be enough) or in a native builtin would let a plugin escape into the whole app. That means the
   Keychain, Accessibility access, the clipboard and the file index. WASM's linear memory is a
   hard boundary checked by the runtime. This is the main reason to move once untrusted
   third-party plugins exist.
2. **No memory limit.** A plugin can allocate until Moo is killed. The deadline bounds time,
   not memory.
3. **Everything runs on the main thread.** A plugin that keeps landing just under 250 ms will
   still make typing stutter. The planned "one worker thread per VM" isn't built.
4. **The bytecode format isn't stable.** `.tishc` is `tishlang_bytecode::serialize` output. Its
   layout and opcode set follow the compiler. A plugin built with one Tish version isn't
   guaranteed to load in a Moo built with another. That's harmless while plugins are rebuilt from
   source in every release. Prebuilt community `.tishc` files would need a format version and a
   rebuild-on-mismatch rule.
5. **Less scrutiny.** WASM runtimes like wasmtime are fuzzed continuously and security-audited.
   `tish_vm` as a sandbox has the tests in `vmplug.rs` and nothing more.
6. **Tied to Tish.** If the Tish-only policy ever changes, `.tishc` can't host anything else.

## What WASM/WASI would offer

- **A hard memory boundary** and **memory limits** per instance (wasmtime `StoreLimits`).
- **Interruption without a cooperating VM:** wasmtime *fuel* (instruction counting) or *epoch
  interruption* (a timer bumps a counter, and generated code checks it). Both work even while
  compiled to native code, so plugins could have a JIT and budgets at the same time.
- **Running off the main thread** comes naturally: a `Store` is `Send`, so each plugin can live
  on a worker.
- **A stable binary format** with a versioned spec, and a standard way to describe interfaces
  (WIT, the Component Model).
- **Any source language**, which is irrelevant under the current policy.

## What Tish's WASM targets actually produce

`tish build --target wasm|wasi` does **not** compile Tish to WebAssembly instructions.
`tish_wasm` compiles to Tish bytecode, then links that bytecode into a WebAssembly build of
**the whole Tish VM** (`tish_wasm_runtime`). Every `.wasm` file is a full VM plus one embedded
chunk. It's also built as a WASI *command* (it runs `main` and exits) rather than a library with
exported functions. It has no `register` or `moo`, so a plugin built this way fails with
`Undefined variable: register`.

Measured:

| | `.tishc` (today) | `tish build --target wasi` |
| --- | --- | --- |
| Dev Toolbox plugin | 24 KB | 3.0 MB |
| `console.log("hi")` | — | 3.0 MB |
| Load / start | 0.1–0.4 ms per plugin (in-process VM) | first run 0.52 s (wasmtime compiling machine code, cached afterwards); 10–20 ms per run after, CLI process included; ~10 ms precompiled (`.cwasm`) |
| Calls into it | direct function call, shared values | none: a command module has no exports |

So "build each plugin to WASI with today's toolchain" would mean 8 plugins × 3 MB, each carrying
its own copy of the same VM, and still no way to call them. **Any real WASM move needs Moo to
define its own guest interface**, whichever source language is used.

## Options

### A. Keep `.tishc`, harden it
- Add a format version to `.tishc`. Refuse, or rebuild from source, on a mismatch.
- Add a per-VM allocation cap to `tish_vm` (fail the call when exceeded).
- Move each VM to a worker thread, posting results to the main thread. This was already planned
  in `plugin-api.md`.
- Fuzz the bytecode deserializer and the `moo` host object.

Cheapest. It doesn't fix the in-process trust problem.

### B. One shared Tish VM in WASM, plugins stay `.tishc` (recommended)
Build `tish_vm` once as a WASM *reactor* module, `moo-plugin-vm.wasm`, that exports
`load(chunk)`, `call(export, args)` and an event pump, and imports Moo's host functions. Moo
embeds a WASM runtime, creates **one instance per plugin** from that single module, and passes in
each plugin's `.tishc` bytes.

- Plugins, the plugins repo, `build.sh` and the authoring experience **don't change**.
- The 3 MB VM is compiled once and shared through wasmtime's module cache. Each plugin instance
  costs its own linear memory, not another VM binary.
- You get WASM's memory boundary, memory limits, fuel/epoch budgets and workers.
- Values cross the boundary as JSON (or a compact encoding) in linear memory. Plugin results are
  already plain data, so this is a small, well-defined layer.

### C. WASM Component Model, any language
Define the plugin contract in WIT (`moo:plugin/plugin` world: `manifest`, `run`, `list`,
`suggest`, …, with host imports for fetch, store, secret, sign-in, refresh, notify, log). Any
language with component tooling can implement it. Tish would need a component target, or it uses
B's VM-in-a-component.

The most open and the most work. It contradicts the Tish-only policy, so it's worth doing only
if that policy changes. Note that B can be packaged as a component later, so B doesn't rule C
out.

### D. Compile Tish directly to WASM instructions
A real code generator (Tish → WASM functions, with GC via WasmGC or a runtime) would make small
modules that need no VM. This is a compiler project at Tish's scale, not a Moo project, so it's
out of scope here.

## Transition plan (A now, then B)

### Phase 0: groundwork (useful whatever happens next)
1. **Write the contract down as a schema.** Turn `plugin-api.md` into a versioned interface:
   - the exports: `manifest`, `run`, `list`, `suggest`, `blocker`, `visible`, `open`, plus view
     events
   - the host calls with argument and result types
   - an `apiVersion` in the manifest
   - WIT syntax works as the source even before any WASM exists
2. **Manifest in `moo.json`** (already planned), so loading never needs to run code to learn the
   commands, and the marketplace can read it.
3. **Version `.tishc`:**
   - a header with the bytecode format version and the Tish version that built it
   - `loadBytecodePlugin` refuses a mismatch with a clear error
4. **Golden tests:** for each plugin in `moomoi/plugins`, a recorded list of calls and expected
   rows, using a mock `moo` like the one used to test the new plugins. Any runtime swap must
   pass these unchanged.

### Phase 1: harden `.tishc` (option A)
- Allocation cap per VM in `tish_vm`, surfaced as "Plugin error: out of memory".
- A worker thread per Tier A VM, with results posted to the main thread. The shell already
  handles plugins answering later (`moo.refresh()`), so `list` and `suggest` can become
  "latest result wins" without UI changes.
- Fuzz `tishlang_bytecode::deserialize` and `pluginhost.rs` (cargo-fuzz).

### Phase 2: the VM moves into WASM (option B)
1. **Pick the runtime.** This is the main macOS decision:
   - **wasmtime with its JIT** (Cranelift) is fastest, but generating code at run time needs the
     `com.apple.security.cs.allow-jit` entitlement under the hardened runtime. Moo doesn't
     request it today (`packaging/entitlements.plist` has only the address book key). It's an
     allowed, notarizable entitlement, but it widens what the app may do.
   - **wasmtime ahead-of-time:** compile `moo-plugin-vm.wasm` to `.cwasm` at build time, sign it
     with the app, and load it with `Module::deserialize`. No JIT entitlement, and startup is
     fast (~10 ms measured). This is the recommended choice, since there's only one module to
     precompile.
   - **An interpreter (wasmtime's Pulley, wasmi):** no JIT and no precompilation, but slower.
     Plugins would then run an interpreter inside an interpreter, which is probably fine for row
     building and worth measuring with the golden tests.
2. **Build the guest.** A new crate (or a `tish_wasm_runtime` feature) builds `tish_vm` for
   `wasm32-wasip1` as a reactor that exports:
   - `alloc` / `free`
   - `load(ptr, len) -> handle` (deserialize a `.tishc`, run its top level, capture the
     `register` call)
   - `call(handle, name_ptr, name_len, args_ptr, args_len) -> result_ptr`
   - `deliver(callback_id, payload_ptr, payload_len)` for fetch and sign-in replies

   It imports one function per host call (`moo_fetch`, `moo_store_get`, …). Inside the guest,
   the `moo` global is a thin Tish shim over those imports. Callbacks become ids: the guest keeps
   `id → closure`, and the host later calls `deliver(id, …)`.
3. **Host side in `moo-macos`.** A `wasmplug.rs` next to `vmplug.rs`, with the same outward API
   (`loadBytecodePlugin` returns a plugin value whose methods the shell calls):
   - **Bridging:** each host import calls the existing `pluginhost.rs` logic (network allowlist,
     store, Keychain, sign-in). That logic stays; only the bridge changes.
   - **Budgets:** epoch interruption for the 1000/250 ms budgets (one timer thread bumps the
     epoch), and `StoreLimits` for memory (say 64 MB per plugin).
   - **Values:** results are JSON-decoded into Tish `Value`s for the shell.
4. **Ship it behind a switch.** `MOO_PLUGIN_RUNTIME=wasm|vm`, defaulting to `vm`. CI runs the
   golden tests and the `vmplug.rs` tests under both.
5. **Flip the default** once the golden tests pass and drive-script runs (`drive-plugins.sh`,
   `drive-convert.sh`, `drive-views.sh`) look the same. Keep the in-process VM for one release as
   a fallback, then remove it.
6. **Packaging:** the signed `.mooplugin` archive (planned) carries `.tishc` + `moo.json` and
   records the bytecode format version. Moo's WASM VM loads any `.tishc` whose version it
   supports, so community plugins become installable without a Moo release.

### Phase 3 (optional): components and other languages
Only if the Tish-only policy changes:
- Publish the Phase 0 WIT as a real component world.
- Wrap the Phase 2 guest as a component.
- Let other languages implement the world directly.
- Tier C in `plugin-api.md` (an untrusted native helper process) becomes unnecessary, because a
  component already runs untrusted code safely.

## What changes where

| Area | Phase 0–1 | Phase 2 |
| --- | --- | --- |
| `moomoi/plugins` | `moo.json` gains `apiVersion` and the command list; golden tests | nothing (still `.tishc`) |
| `tish_bytecode` | format version header | — |
| `tish_vm` | allocation cap | builds for `wasm32-wasip1` as a reactor |
| `packages/moo-macos` | worker threads in `vmplug.rs` | new `wasmplug.rs`, a wasmtime dependency, `.cwasm` built at app build time |
| `pluginhost.rs` | unchanged | called from WASM imports instead of Tish natives |
| `scripts/bundle-macos.sh` | — | copy and sign `moo-plugin-vm.cwasm` |
| Entitlements | — | none if precompiled; `allow-jit` only if JIT-compiling at run time |
| App size | — | about +3 MB for the VM module, plus the wasmtime runtime in the binary |

## Open questions

- **Runtime weight.** How much does wasmtime add to the 14 MB `moo` binary? It needs measuring.
  wasmi is far smaller if Pulley or wasmtime is too heavy.
- **View plugins.** `@moo/ui` trees and their event handlers cross the boundary often. Measure
  JSON round trips for Hello List before committing.
- **Sign-in.** `moo.signIn` holds a loopback listener and calls back much later. With callback
  ids this is fine, but the guest must survive being idle, which an instance kept alive allows.
- **Tier B.** It stays a native, signed, first-party exception, or folds into the WASM VM once
  `utils` is rewritten as Tier A. It uses nothing a Tier A plugin can't do.
- **When.** The trigger for Phase 2 is the first plugin Moo loads that it didn't build from
  source: the planned marketplace Install-without-a-release flow. Until then, Phase 0 and Phase 1
  give most of the safety for a fraction of the work.
