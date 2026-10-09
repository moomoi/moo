//! `moo-macos`: native launcher services for the Moo shell, imported from Tish as
//! `import { reindex, search, launch, setup, ... } from "moo-macos"`.

/// Release builds set `MOO_VERSION` (scripts/build-universal.sh); local builds say "dev".
pub const VERSION: &str = match option_env!("MOO_VERSION") {
    Some(v) => v,
    None => "dev",
};

#[cfg(target_os = "macos")]
mod ai;
#[cfg(any(target_os = "macos", windows))]
mod bridge;
#[cfg(any(target_os = "macos", windows))]
mod cli;
#[cfg(any(target_os = "macos", windows))]
mod ipc;
mod frecency;
mod fsindex;
#[cfg(target_os = "macos")]
mod fslive;
#[cfg(windows)]
#[path = "fslive_win.rs"]
mod fslive;
#[cfg(any(target_os = "macos", windows))]
mod http;
mod index;
#[cfg(target_os = "macos")]
mod keychain;
#[cfg(windows)]
#[path = "keychain_win.rs"]
mod keychain;
#[cfg(windows)]
mod win;

/// What the plugin host and the bridge need from the platform: `launch`, `debug_log`, `with_ui`
/// (mac.rs on macOS, win.rs on Windows).
#[cfg(any(target_os = "macos", windows))]
mod sys {
    #[cfg(target_os = "macos")]
    pub(crate) use crate::mac::{debug_log, launch, with_ui};
    #[cfg(windows)]
    pub(crate) use crate::win::{debug_log, launch, with_ui};
}
#[cfg(target_os = "macos")]
mod keymap;
#[cfg(target_os = "macos")]
mod keys;
#[cfg(target_os = "macos")]
#[cfg(target_os = "macos")]
mod mac;
#[cfg(any(target_os = "macos", windows))]
mod oauth;
#[cfg(any(target_os = "macos", windows))]
mod pluginhost;
#[cfg(target_os = "macos")]
#[cfg(target_os = "macos")]
mod infoplist;
mod snippets;
#[cfg(target_os = "macos")]
mod theme;
mod vmplug;

use std::sync::Arc;

use tishlang_core::{ObjectMap, Value, VmRef};

fn str_arg(args: &[Value], i: usize) -> String {
    match args.get(i) {
        Some(Value::String(s)) => s.to_string(),
        _ => String::new(),
    }
}

fn num_arg(args: &[Value], i: usize, default: f64) -> f64 {
    match args.get(i) {
        Some(Value::Number(n)) => *n,
        _ => default,
    }
}

pub(crate) fn obj(pairs: Vec<(&str, Value)>) -> Value {
    let mut m = ObjectMap::default();
    for (k, v) in pairs {
        m.insert(Arc::from(k), v);
    }
    Value::object(m)
}

fn field(v: Option<&Value>, key: &str) -> Option<Value> {
    match v {
        Some(Value::Object(o)) => o.borrow().strings.get(key).cloned(),
        _ => None,
    }
}

fn native_reindex(_args: &[Value]) -> Value {
    let (count, ms) = index::reindex();
    obj(vec![("count", Value::Number(count as f64)), ("ms", Value::Number(ms))])
}

/// `fuzzy(query, titles, limit)` -> `[{ index, score }]`, best first.
fn native_fuzzy(args: &[Value]) -> Value {
    let titles: Vec<String> = match args.get(1) {
        Some(Value::Array(a)) => a
            .borrow()
            .iter()
            .map(|v| match v {
                Value::String(s) => s.to_string(),
                _ => String::new(),
            })
            .collect(),
        _ => Vec::new(),
    };
    let limit = num_arg(args, 2, titles.len() as f64).max(0.0) as usize;
    let rows: Vec<Value> = index::fuzzy(&str_arg(args, 0), &titles, limit)
        .into_iter()
        .map(|(i, s)| obj(vec![("index", Value::Number(i as f64)), ("score", Value::Number(s as f64))]))
        .collect();
    Value::Array(VmRef::new(rows))
}

/// `pluginPaths(dir)` -> paths of loadable plugin modules in `dir`.
fn native_plugin_paths(args: &[Value]) -> Value {
    let rows: Vec<Value> = index::plugin_paths(&str_arg(args, 0))
        .into_iter()
        .map(|p| Value::String(p.as_str().into()))
        .collect();
    Value::Array(VmRef::new(rows))
}

/// `loadBytecodePlugin(path)` -> `{ ok, plugin, ms }` or `{ ok: false, error }`.
fn native_load_bytecode_plugin(args: &[Value]) -> Value {
    match vmplug::load(&str_arg(args, 0)) {
        Ok((plugin, ms)) => obj(vec![("ok", Value::Bool(true)), ("plugin", plugin), ("ms", Value::Number(ms))]),
        Err(e) => obj(vec![("ok", Value::Bool(false)), ("error", Value::String(e.as_str().into()))]),
    }
}

/// `bundleResources()` -> `Moo.app/Contents/Resources` when running from an app bundle, else null.
/// The running binary's path, resolved. On Windows without the `\\?\` prefix canonicalize
/// adds: such paths take no forward slashes, and Tish code joins paths with `/`.
fn current_exe() -> Option<std::path::PathBuf> {
    let p = std::env::current_exe().ok()?.canonicalize().ok()?;
    #[cfg(windows)]
    if let Some(s) = p.to_str().and_then(|s| s.strip_prefix(r"\\?\")) {
        return Some(std::path::PathBuf::from(s));
    }
    Some(p)
}

#[cfg(windows)]
fn native_bundle_resources(_args: &[Value]) -> Value {
    // An installed Moo is moo.exe with plugins\ beside it; a dev build has neither.
    match current_exe().as_deref().and_then(|p| p.parent()).filter(|d| d.join("plugins").is_dir()) {
        Some(d) => Value::String(d.to_string_lossy().as_ref().into()),
        None => Value::Null,
    }
}

#[cfg(not(windows))]
fn native_bundle_resources(_args: &[Value]) -> Value {
    let exe = current_exe();
    let contents = exe.as_deref().and_then(|p| p.parent()).filter(|d| d.ends_with("MacOS")).and_then(|d| d.parent());
    match contents {
        Some(c) if c.ends_with("Contents") && c.parent().is_some_and(|b| b.extension().is_some_and(|e| e == "app")) => {
            Value::String(c.join("Resources").to_string_lossy().as_ref().into())
        }
        _ => Value::Null,
    }
}

/// `exeDir()` -> the folder holding the running binary, so paths don't depend on the working
/// directory (the CLI starts the app from that folder).
fn native_exe_dir(_args: &[Value]) -> Value {
    let exe = current_exe();
    match exe.as_deref().and_then(|p| p.parent()) {
        Some(d) => Value::String(d.to_string_lossy().as_ref().into()),
        None => Value::String(".".into()),
    }
}

/// `about()` -> `{ version, exe, app }`: the version this build was made as ("dev" for local
/// builds), the running binary, and the `.app` bundle holding it (null outside one). For debugging
/// which Moo is running.
fn native_about(_args: &[Value]) -> Value {
    let exe = current_exe();
    let app = exe
        .as_deref()
        .and_then(|p| p.ancestors().find(|a| a.extension().is_some_and(|e| e == "app")))
        .map(|p| Value::String(p.to_string_lossy().as_ref().into()))
        .unwrap_or(Value::Null);
    let exe = exe.map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    let mut m = ObjectMap::default();
    m.insert(Arc::from("version"), Value::String(VERSION.into()));
    m.insert(Arc::from("exe"), Value::String(exe.as_str().into()));
    m.insert(Arc::from("app"), app);
    Value::object(m)
}

/// `recordUse(key)`: count one open of an app path or command key toward its frecency.
fn native_record_use(args: &[Value]) -> Value {
    frecency::record(&str_arg(args, 0));
    Value::Null
}

/// `frecency(key)` -> score bonus to add to a fuzzy match score (0 when never used).
fn native_frecency(args: &[Value]) -> Value {
    Value::Number(frecency::boost(&str_arg(args, 0)) as f64)
}

/// `appFolders()` -> the folders the app index scans (for watching them).
fn native_app_folders(_args: &[Value]) -> Value {
    Value::Array(VmRef::new(index::root_strings().iter().map(|p| Value::String(p.as_str().into())).collect()))
}

/// `appPaths()` -> every indexed app's path, A–Z.
fn native_app_paths(_args: &[Value]) -> Value {
    Value::Array(VmRef::new(index::paths().iter().map(|p| Value::String(p.as_str().into())).collect()))
}

fn native_app_count(_args: &[Value]) -> Value {
    Value::Number(index::app_count() as f64)
}

fn native_search(args: &[Value]) -> Value {
    let limit = num_arg(args, 1, 8.0).max(0.0) as usize;
    let rows: Vec<Value> = index::search(&str_arg(args, 0), limit)
        .into_iter()
        .map(|(app, score)| {
            obj(vec![
                ("name", Value::String(app.name.as_str().into())),
                ("path", Value::String(app.path.as_str().into())),
                ("kind", Value::String("Application".into())),
                ("score", Value::Number(score as f64)),
            ])
        })
        .collect();
    Value::Array(VmRef::new(rows))
}

#[cfg(target_os = "macos")]
mod natives {
    use super::*;
    use dispatch2::DispatchQueue;

    /// `setup({ width, height, onKey, onShow, onHotkey, hidden })`: call before `macos.run(App)`.
    /// `hidden` keeps the panel closed at startup.
    pub fn setup(args: &[Value]) -> Value {
        let opts = args.first();
        let w = field(opts, "width").and_then(|v| v.as_number()).unwrap_or(720.0);
        let h = field(opts, "height").and_then(|v| v.as_number()).unwrap_or(440.0);
        mac::set_panel_size(w, h);
        mac::set_callbacks(field(opts, "onKey"), field(opts, "onShow"), field(opts, "onHotkey"));
        mac::set_start_hidden(matches!(field(opts, "hidden"), Some(Value::Bool(true))));
        mac::schedule_setup();
        Value::Null
    }

    /// `setTheme(theme)`: the panel's colours, radius, margins, position and motion (see
    /// app/src/theme.tish). Call before `setup`; calling it again restyles the open panel.
    pub fn set_theme(args: &[Value]) -> Value {
        if let Some(t) = args.first() {
            crate::theme::set(t);
            mac::theme_changed();
        }
        Value::Null
    }

    /// `setArrowKeys(on)`: while on, ← and → go to `onKey` as "left" / "right" (moving through a
    /// grid) instead of moving the search field's cursor.
    pub fn set_arrow_keys(args: &[Value]) -> Value {
        mac::set_arrow_keys(matches!(args.first(), Some(Value::Bool(true))));
        Value::Null
    }

    /// `typeText(text)`: test hook behind `moo type`; inserts `text` into the focused field.
    pub fn type_text(args: &[Value]) -> Value {
        let text = str_arg(args, 0);
        dispatch2::DispatchQueue::main().exec_async(move || mac::type_text(text));
        Value::Null
    }

    /// `setPanelShape(height, segments)`: resize the panel (its top edge stays put) and cut it into
    /// rounded glass pieces, each `[x, width, radius]` spanning the full height (Spotlight's idle
    /// bar is a field capsule plus round buttons). The pieces are only the backdrop; the Tish view
    /// draws everything on them. The layout keeps its full size and is clipped.
    pub fn set_panel_shape(args: &[Value]) -> Value {
        let h = num_arg(args, 0, 0.0);
        let segs: Vec<mac::Piece> = match args.get(1) {
            Some(Value::Array(a)) => a
                .borrow()
                .iter()
                .filter_map(|s| match s {
                    Value::Array(p) => {
                        let p = p.borrow();
                        let n = |i: usize| p.get(i).and_then(|v| v.as_number());
                        Some(mac::Piece { x: n(0)?, width: n(1)?, radius: n(2).unwrap_or(0.0) })
                    }
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        if h > 0.0 {
            mac::set_panel_shape(h, segs);
        }
        Value::Null
    }

    /// `registerHotkey(spec, action?)` -> `{ ok, id, display, registered }` or `{ ok: false, error }`.
    /// `spec` names keys as printed ("cmd+space"); `registered` lists what was registered after the
    /// keyboards' modifier mappings (["ctrl+space"] with Command and Control swapped); `display` is
    /// "⌘Space". Without `action` the hotkey toggles the launcher; with one, pressing it calls
    /// `onHotkey(action)`. `label` names what it runs in conflict messages.
    pub fn register_hotkey(args: &[Value]) -> Value {
        let action = match args.get(1) {
            Some(Value::String(s)) if !s.is_empty() => s.to_string(),
            _ => mac::TOGGLE.to_string(),
        };
        match mac::register_hotkey(&str_arg(args, 0), &action, &str_arg(args, 2)) {
            Ok(h) => obj(vec![
                ("ok", Value::Bool(true)),
                ("id", Value::Number(h.id as f64)),
                ("display", Value::String(h.display.as_str().into())),
                (
                    "registered",
                    Value::Array(VmRef::new(h.registered.iter().map(|s| Value::String(s.as_str().into())).collect())),
                ),
            ]),
            Err(e) => obj(vec![("ok", Value::Bool(false)), ("error", Value::String(e.as_str().into()))]),
        }
    }

    pub fn unregister_hotkey(args: &[Value]) -> Value {
        Value::Bool(mac::unregister_hotkey(num_arg(args, 0, 0.0) as u32))
    }

    /// `checkHotkey(spec)` -> `{ ok, display }` when it could be bound now, else `{ ok: false, error }`.
    pub fn check_hotkey(args: &[Value]) -> Value {
        match mac::check_hotkey(&str_arg(args, 0)) {
            Ok(d) => obj(vec![("ok", Value::Bool(true)), ("display", Value::String(d.as_str().into()))]),
            Err(e) => obj(vec![("ok", Value::Bool(false)), ("error", Value::String(e.as_str().into()))]),
        }
    }

    /// `hotkeyDisplay(spec)` -> "⌘⇧G", or the spec itself when it does not parse.
    pub fn hotkey_display(args: &[Value]) -> Value {
        let spec = str_arg(args, 0);
        let d = keys::parse(&spec).map(|s| keys::display(s.mods, s.key)).unwrap_or(spec);
        Value::String(d.as_str().into())
    }

    /// `recordHotkey(on)`: while on, panel keypresses arrive as `onKey("record:<spec>")`.
    pub fn record_hotkey(args: &[Value]) -> Value {
        mac::set_recording(matches!(args.first(), Some(Value::Bool(true))));
        Value::Null
    }

    // ── Shortcuts ──

    fn s(v: &str) -> Value {
        Value::String(v.into())
    }

    fn arr(items: Vec<Value>) -> Value {
        Value::Array(VmRef::new(items))
    }

    /// `isRightClick()`: whether the onClick running now came from a right-click or control-click.
    pub fn is_right_click(_a: &[Value]) -> Value {
        Value::Bool(mac::is_right_click())
    }

    /// `setPanelKeys([spec])`: while the panel has focus these keys arrive as
    /// `onKey("panel:<spec>")` instead of reaching the search field.
    pub fn set_panel_keys(args: &[Value]) -> Value {
        let specs: Vec<String> = match args.first() {
            Some(Value::Array(a)) => a.borrow().iter().filter_map(|v| if let Value::String(x) = v { Some(x.to_string()) } else { None }).collect(),
            _ => Vec::new(),
        };
        let parsed = specs.iter().filter_map(|x| keys::parse(x).ok()).map(|p| (p.mods, p.code)).collect();
        mac::set_panel_keys(parsed);
        Value::Null
    }

    /// `keysError(spec)` -> null, or why "cmd+shift+k" style `spec` doesn't parse.
    pub fn keys_error(args: &[Value]) -> Value {
        match keys::parse(&str_arg(args, 0)) {
            Ok(_) => Value::Null,
            Err(e) => s(&e),
        }
    }

    /// `localDateTime()` -> `["2026-10-03", "17:20"]` in the local time zone.
    pub fn local_date_time(_a: &[Value]) -> Value {
        let (d, t) = mac::local_date_time();
        arr(vec![s(&d), s(&t)])
    }
    // ── Command line ──

    /// `cliMain()`: when this process was started as a command (`moo files foo`), or another
    /// Moo is already running, act as its client and exit. Otherwise return false: be the app.
    pub fn cli_main(args: &[Value]) -> Value {
        let a = cli::args();
        if !a.is_empty() || cli::running() {
            std::process::exit(cli::client(&a, &str_arg(args, 0)));
        }
        Value::Bool(false)
    }

    /// `cliServe(onCli)`: answer `moo` commands. `onCli(args, cwd, token)` runs on the main
    /// thread and replies with `cliWrite(token, text, "out"|"err")` and `cliEnd(token, code)`.
    pub fn cli_serve(args: &[Value]) -> Value {
        cli::set_handler(args.first().cloned());
        let served = cli::serve(|req: cli::Request| {
            DispatchQueue::main().exec_async(move || {
                let Some(Value::Function(f)) = cli::handler() else {
                    cli::end(req.token, 1);
                    return;
                };
                let argv = arr(req.args.iter().map(|a| s(a)).collect());
                crate::mac::with_ui(|| {
                    let _ = f.call(&[argv, s(&req.cwd), Value::Number(req.token as f64)]);
                });
            });
        });
        match served {
            Ok(p) => obj(vec![("ok", Value::Bool(true)), ("path", s(&p.to_string_lossy()))]),
            Err(e) => obj(vec![("ok", Value::Bool(false)), ("error", s(&e))]),
        }
    }

    pub fn cli_write(args: &[Value]) -> Value {
        let stream = str_arg(args, 2);
        Value::Bool(cli::write(num_arg(args, 0, 0.0) as u64, if stream.is_empty() { "out" } else { &stream }, &str_arg(args, 1)))
    }

    pub fn cli_end(args: &[Value]) -> Value {
        Value::Bool(cli::end(num_arg(args, 0, 0.0) as u64, num_arg(args, 1, 0.0) as i32))
    }

    pub fn show(_a: &[Value]) -> Value {
        mac::show();
        Value::Null
    }

    pub fn hide(_a: &[Value]) -> Value {
        mac::hide();
        Value::Null
    }

    pub fn toggle(_a: &[Value]) -> Value {
        mac::toggle();
        Value::Null
    }

    pub fn quit(_a: &[Value]) -> Value {
        mac::quit();
        Value::Null
    }

    /// `fileIndexStart()`: build the file index in the background (snapshot or crawl) and keep it
    /// live. Returns at once.
    pub fn file_index_start(_args: &[Value]) -> Value {
        Value::Bool(fslive::start())
    }

    fn file_rows(hits: Vec<fsindex::FileHit>) -> Value {
        let rows: Vec<Value> = hits
            .into_iter()
            .map(|h| {
                obj(vec![
                    ("name", Value::String(h.name.as_str().into())),
                    ("path", Value::String(h.path.as_str().into())),
                    ("icon", Value::String("".into())),
                    ("kind", Value::String(if h.is_dir { "Folder" } else { "File" }.into())),
                    ("detail", Value::String(h.detail.as_str().into())),
                    ("score", Value::Number(h.score as f64)),
                ])
            })
            .collect();
        Value::Array(VmRef::new(rows))
    }

    /// `findFiles(query, limit, icons = 8)` -> `{ ready, results, ms }`, synchronous. `ready` is
    /// false while the index builds (or is briefly busy); icons are resolved for the first `icons`.
    pub fn find_files(args: &[Value]) -> Value {
        let limit = num_arg(args, 1, 8.0).max(0.0) as usize;
        match fslive::search(&str_arg(args, 0), limit) {
            Some((hits, ms)) => obj(vec![
                ("ready", Value::Bool(true)),
                ("results", file_rows(hits)),
                ("ms", Value::Number(ms)),
            ]),
            None => obj(vec![("ready", Value::Bool(false)), ("results", Value::Array(VmRef::new(vec![]))), ("ms", Value::Number(0.0))]),
        }
    }

    /// `recentFiles(limit)`: files and folders opened through Moo, most used first.
    pub fn recent_files(args: &[Value]) -> Value {
        let limit = num_arg(args, 0, 8.0).max(0.0) as usize;
        file_rows(fslive::recent(limit))
    }

    /// `quickLook(path)` -> whether the preview is open: shows `path` (or switches to it), or
    /// closes the preview when `path` is empty.
    pub fn quick_look(args: &[Value]) -> Value {
        Value::Bool(mac::quick_look(&str_arg(args, 0)))
    }

    pub fn quick_look_visible(_a: &[Value]) -> Value {
        Value::Bool(mac::quick_look_visible())
    }

    /// `watchTyped(keywords, cb)`: call `cb(keyword)` when one of the keywords is typed in another
    /// app (needs Accessibility, which platform.tish asks for).
    pub fn watch_typed(args: &[Value]) -> Value {
        let keywords: Vec<String> = match args.first() {
            Some(Value::Array(a)) => a.borrow().iter().filter_map(|v| if let Value::String(k) = v { Some(k.to_string()) } else { None }).collect(),
            _ => Vec::new(),
        };
        mac::watch_snippets(keywords, args.get(1).cloned().unwrap_or(Value::Null));
        Value::Null
    }

    /// `fileIndexStatus()` -> `{ state, entries, folders, bytes, buildMs, fromSnapshot, updates }`.
    pub fn file_index_status(_args: &[Value]) -> Value {
        let s = fslive::status();
        obj(vec![
            ("state", Value::String(s.state.into())),
            ("entries", Value::Number(s.entries as f64)),
            ("folders", Value::Number(s.dirs as f64)),
            ("bytes", Value::Number(s.bytes as f64)),
            ("buildMs", Value::Number(s.build_ms)),
            ("fromSnapshot", Value::Bool(s.from_snapshot)),
            ("updates", Value::Number(s.updates as f64)),
        ])
    }

    /// `aiAvailability()` -> `"available"`, or why Apple's on-device model cannot answer.
    pub fn ai_availability(_a: &[Value]) -> Value {
        match ai::availability() {
            Ok(()) => Value::String("available".into()),
            Err(reason) => Value::String(reason.as_str().into()),
        }
    }

    /// `aiSession(instructions)` -> session id (0 when unavailable). Asks in a session share context.
    /// `aiSession(instructions, toolsJson?, onTool?)`; see `ai::session`.
    pub fn ai_session(args: &[Value]) -> Value {
        let tools = match args.get(1) {
            Some(Value::String(s)) => s.to_string(),
            _ => "[]".to_string(),
        };
        let on_tool = args.get(2).filter(|v| matches!(v, Value::Function(_))).cloned();
        Value::Number(ai::session(&str_arg(args, 0), &tools, on_tool) as f64)
    }

    pub fn ai_end_session(args: &[Value]) -> Value {
        ai::end_session(num_arg(args, 0, 0.0) as u64);
        Value::Null
    }

    pub fn ai_prewarm(args: &[Value]) -> Value {
        ai::prewarm(num_arg(args, 0, 0.0) as u64);
        Value::Null
    }

    /// `aiAsk(session, prompt, onEvent)` -> request id (0 if the session is busy or unknown).
    /// `onEvent({ request, kind, text })` runs on the main thread; kind is "partial" (whole reply so
    /// far), then one of "done", "error" or "cancelled".
    pub fn ai_ask(args: &[Value]) -> Value {
        let cb = args.get(2).cloned().unwrap_or(Value::Null);
        Value::Number(ai::ask(num_arg(args, 0, 0.0) as u64, &str_arg(args, 1), cb) as f64)
    }

    pub fn ai_cancel(args: &[Value]) -> Value {
        ai::cancel(num_arg(args, 0, 0.0) as u64);
        Value::Null
    }

    fn callback(args: &[Value], i: usize) -> Option<Value> {
        args.get(i).filter(|v| matches!(v, Value::Function(_))).cloned()
    }
    // ── Keychain, OAuth and UI callbacks for the app's ai.tish ──

    fn field_text(v: Option<&Value>, key: &str) -> String {
        match v {
            Some(Value::Object(o)) => match o.borrow().strings.get(key) {
                Some(Value::String(x)) => x.to_string(),
                _ => String::new(),
            },
            _ => String::new(),
        }
    }

    /// `keychainGet(account)` -> the saved secret, or null.
    pub fn keychain_get(args: &[Value]) -> Value {
        crate::keychain::get(&str_arg(args, 0)).map_or(Value::Null, |v| s(&v))
    }

    /// `keychainSet(account, secret)` -> null, or why it couldn't be saved.
    pub fn keychain_set(args: &[Value]) -> Value {
        match crate::keychain::set(&str_arg(args, 0), &str_arg(args, 1)) {
            Ok(()) => Value::Null,
            Err(e) => s(&e),
        }
    }

    pub fn keychain_delete(args: &[Value]) -> Value {
        Value::Bool(crate::keychain::delete(&str_arg(args, 0)))
    }

    /// `keychainHas(account)`: whether a secret is saved, without reading it (no Keychain prompt).
    pub fn keychain_has(args: &[Value]) -> Value {
        Value::Bool(crate::keychain::has(&str_arg(args, 0)))
    }

    fn endpoints_arg(v: Option<&Value>) -> oauth::Endpoints {
        oauth::Endpoints {
            authorize: field_text(v, "authorize"),
            token: field_text(v, "token"),
            client_id: field_text(v, "clientId"),
            scope: field_text(v, "scope"),
            relay: field_text(v, "relay"),
            extra: Vec::new(),
        }
    }

    fn tokens_value(r: Result<oauth::Tokens, String>) -> Value {
        match r {
            Ok(t) => obj(vec![("access", s(&t.access)), ("refresh", s(&t.refresh)), ("expires", Value::Number(t.expires_at)), ("error", s(""))]),
            Err(e) => obj(vec![("error", s(&e))]),
        }
    }

    static OAUTH_CANCEL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

    /// `oauthSignIn({ authorize, token, clientId, scope, relay }, cb)`: browser sign-in through the
    /// loopback redirect; `cb({ access, refresh, expires, error })` on the main thread.
    pub fn oauth_sign_in(args: &[Value]) -> Value {
        let ep = endpoints_arg(args.first());
        let request = bridge::hold(callback(args, 1));
        OAUTH_CANCEL.store(false, std::sync::atomic::Ordering::Relaxed);
        std::thread::spawn(move || {
            let r = oauth::login(
                &ep,
                |url| {
                    let url = url.to_string();
                    bridge::on_main(move || {
                        mac::launch(&url);
                    });
                },
                &OAUTH_CANCEL,
            );
            bridge::post(request, r, tokens_value, true);
        });
        Value::Null
    }

    pub fn oauth_cancel(_a: &[Value]) -> Value {
        OAUTH_CANCEL.store(true, std::sync::atomic::Ordering::Relaxed);
        Value::Null
    }

    /// `oauthRefresh({ token, clientId }, refreshToken, cb)`: a new access token;
    /// `cb({ access, refresh, expires, error })`. Native so the token request never follows a
    /// redirect.
    pub fn oauth_refresh(args: &[Value]) -> Value {
        let ep = endpoints_arg(args.first());
        let refresh = str_arg(args, 1);
        let request = bridge::hold(callback(args, 2));
        std::thread::spawn(move || {
            let r = oauth::refresh(&ep, &refresh);
            bridge::post(request, r, tokens_value, true);
        });
        Value::Null
    }

    /// `withUi(fn)`: run `fn` against the launcher's UI root and give focus back after, as native
    /// callbacks do (for callbacks from `macos.whenSettled`).
    pub fn with_ui(args: &[Value]) -> Value {
        match args.first() {
            Some(Value::Function(f)) => {
                let f = f.clone();
                mac::with_ui(|| f.call(&[]))
            }
            _ => Value::Null,
        }
    }

    /// `onPluginRefresh(cb)`: `cb()` whenever a plugin calls `moo.refresh()` (new data arrived).
    pub fn on_plugin_refresh(args: &[Value]) -> Value {
        pluginhost::on_refresh(callback(args, 0));
        Value::Null
    }
}

#[cfg(not(target_os = "macos"))]
mod natives {
    use super::*;
    pub fn setup(_a: &[Value]) -> Value { Value::Null }
    pub fn set_panel_shape(_a: &[Value]) -> Value { Value::Null }
    pub fn set_arrow_keys(_a: &[Value]) -> Value { Value::Null }
    pub fn set_theme(_a: &[Value]) -> Value { Value::Null }
    pub fn type_text(_a: &[Value]) -> Value { Value::Null }
    pub fn register_hotkey(_a: &[Value]) -> Value {
        obj(vec![("ok", Value::Bool(false)), ("error", Value::String("macOS only".into()))])
    }
    pub fn show(_a: &[Value]) -> Value { Value::Null }
    pub fn hide(_a: &[Value]) -> Value { Value::Null }
    pub fn unsupported(_a: &[Value]) -> Value {
        obj(vec![("ok", Value::Bool(false)), ("error", Value::String("macOS only".into()))])
    }
    pub use unsupported as unregister_hotkey;
    pub use unsupported as check_hotkey;
    pub fn set_panel_keys(_a: &[Value]) -> Value { Value::Null }
    pub fn is_right_click(_a: &[Value]) -> Value { Value::Bool(false) }
    #[cfg(not(windows))]
    pub use unsupported as cli_serve;
    pub fn hotkey_display(a: &[Value]) -> Value { Value::String(str_arg(a, 0).as_str().into()) }
    pub fn toggle(_a: &[Value]) -> Value { Value::Null }
    pub fn record_hotkey(_a: &[Value]) -> Value { Value::Null }
    pub fn keys_error(_a: &[Value]) -> Value { Value::Null }
    pub fn local_date_time(_a: &[Value]) -> Value { Value::Array(VmRef::new(vec![Value::String("".into()), Value::String("".into())])) }
    #[cfg(windows)]
    pub fn cli_main(args: &[Value]) -> Value {
        let a = crate::cli::args();
        if !a.is_empty() || crate::cli::running() {
            std::process::exit(crate::cli::client(&a, &str_arg(args, 0)));
        }
        Value::Bool(false)
    }
    /// `cliServe(onCli)`: answer `moo` commands; `onCli(args, cwd, token)` on the UI thread.
    #[cfg(windows)]
    pub fn cli_serve(args: &[Value]) -> Value {
        crate::cli::set_handler(args.first().cloned());
        crate::win::ensure_window();
        let served = crate::cli::serve(|req: crate::cli::Request| {
            crate::win::on_main(move || {
                let Some(Value::Function(f)) = crate::cli::handler() else {
                    crate::cli::end(req.token, 1);
                    return;
                };
                let argv = Value::Array(VmRef::new(req.args.iter().map(|a| Value::String(a.as_str().into())).collect()));
                let _ = f.call(&[argv, Value::String(req.cwd.as_str().into()), Value::Number(req.token as f64)]);
            });
        });
        match served {
            Ok(p) => obj(vec![("ok", Value::Bool(true)), ("path", Value::String(p.to_string_lossy().as_ref().into()))]),
            Err(e) => obj(vec![("ok", Value::Bool(false)), ("error", Value::String(e.into()))]),
        }
    }
    #[cfg(windows)]
    pub fn cli_write(args: &[Value]) -> Value {
        let stream = str_arg(args, 2);
        Value::Bool(crate::cli::write(num_arg(args, 0, 0.0) as u64, if stream.is_empty() { "out" } else { &stream }, &str_arg(args, 1)))
    }
    #[cfg(windows)]
    pub fn cli_end(args: &[Value]) -> Value {
        Value::Bool(crate::cli::end(num_arg(args, 0, 0.0) as u64, num_arg(args, 1, 0.0) as i32))
    }
    #[cfg(not(windows))]
    pub fn cli_main(_a: &[Value]) -> Value { Value::Bool(false) }
    #[cfg(not(windows))]
    pub fn cli_write(_a: &[Value]) -> Value { Value::Bool(false) }
    #[cfg(not(windows))]
    pub fn cli_end(_a: &[Value]) -> Value { Value::Bool(false) }
    pub fn quit(_a: &[Value]) -> Value { Value::Null }
    #[cfg(windows)]
    pub fn file_index_start(_a: &[Value]) -> Value { Value::Bool(fslive::start()) }
    #[cfg(not(windows))]
    pub fn file_index_start(_a: &[Value]) -> Value { Value::Bool(false) }
    #[cfg(windows)]
    fn file_rows(hits: Vec<crate::fsindex::FileHit>) -> Value {
        let rows: Vec<Value> = hits
            .into_iter()
            .map(|h| {
                obj(vec![
                    ("name", Value::String(h.name.as_str().into())),
                    ("path", Value::String(h.path.as_str().into())),
                    ("icon", Value::String("".into())),
                    ("kind", Value::String(if h.is_dir { "Folder" } else { "File" }.into())),
                    ("detail", Value::String(h.detail.as_str().into())),
                    ("score", Value::Number(h.score as f64)),
                ])
            })
            .collect();
        Value::Array(VmRef::new(rows))
    }
    #[cfg(windows)]
    pub fn find_files(args: &[Value]) -> Value {
        let limit = num_arg(args, 1, 8.0).max(0.0) as usize;
        match fslive::search(&str_arg(args, 0), limit) {
            Some((hits, ms)) => obj(vec![("ready", Value::Bool(true)), ("results", file_rows(hits)), ("ms", Value::Number(ms))]),
            None => obj(vec![("ready", Value::Bool(false)), ("results", Value::Array(VmRef::new(vec![]))), ("ms", Value::Number(0.0))]),
        }
    }
    #[cfg(windows)]
    pub fn recent_files(args: &[Value]) -> Value {
        file_rows(fslive::recent(num_arg(args, 0, 8.0).max(0.0) as usize))
    }
    #[cfg(not(windows))]
    pub fn find_files(_a: &[Value]) -> Value {
        obj(vec![("ready", Value::Bool(false)), ("results", Value::Array(VmRef::new(vec![]))), ("ms", Value::Number(0.0))])
    }
    #[cfg(not(windows))]
    pub fn recent_files(_a: &[Value]) -> Value { Value::Array(VmRef::new(vec![])) }
    /// `watchTyped(keywords, cb)`: `cb(keyword)` when one is typed in another app.
    #[cfg(windows)]
    pub fn watch_typed(args: &[Value]) -> Value {
        let keywords: Vec<String> = match args.first() {
            Some(Value::Array(a)) => a.borrow().iter().filter_map(|v| if let Value::String(k) = v { Some(k.to_string()) } else { None }).collect(),
            _ => Vec::new(),
        };
        crate::win::watch_snippets(keywords, args.get(1).cloned().unwrap_or(Value::Null));
        Value::Null
    }
    #[cfg(not(windows))]
    pub fn watch_typed(_a: &[Value]) -> Value { Value::Null }
    pub fn quick_look(_a: &[Value]) -> Value { Value::Bool(false) }
    pub fn quick_look_visible(_a: &[Value]) -> Value { Value::Bool(false) }
    #[cfg(windows)]
    pub fn file_index_status(_a: &[Value]) -> Value {
        let s = fslive::status();
        obj(vec![
            ("state", Value::String(s.state.into())),
            ("entries", Value::Number(s.entries as f64)),
            ("folders", Value::Number(s.dirs as f64)),
            ("bytes", Value::Number(s.bytes as f64)),
            ("buildMs", Value::Number(s.build_ms)),
            ("fromSnapshot", Value::Bool(s.from_snapshot)),
            ("updates", Value::Number(s.updates as f64)),
        ])
    }
    #[cfg(not(windows))]
    pub fn file_index_status(_a: &[Value]) -> Value { obj(vec![("state", Value::String("idle".into()))]) }
    pub fn ai_availability(_a: &[Value]) -> Value { Value::String("macOS only".into()) }
    pub fn ai_session(_a: &[Value]) -> Value { Value::Number(0.0) }
    pub fn ai_end_session(_a: &[Value]) -> Value { Value::Null }
    pub fn ai_prewarm(_a: &[Value]) -> Value { Value::Null }
    pub fn ai_ask(_a: &[Value]) -> Value { Value::Number(0.0) }
    pub fn ai_cancel(_a: &[Value]) -> Value { Value::Null }
    pub fn keychain_get(_a: &[Value]) -> Value { Value::Null }
    pub fn keychain_set(_a: &[Value]) -> Value { Value::String("macOS only".into()) }
    pub fn keychain_delete(_a: &[Value]) -> Value { Value::Bool(false) }
    pub fn keychain_has(_a: &[Value]) -> Value { Value::Bool(false) }
    pub fn oauth_sign_in(_a: &[Value]) -> Value { Value::Null }
    pub fn oauth_cancel(_a: &[Value]) -> Value { Value::Null }
    pub fn oauth_refresh(_a: &[Value]) -> Value { Value::Null }
    pub fn with_ui(args: &[Value]) -> Value {
        match args.first() {
            Some(Value::Function(f)) => f.call(&[]),
            _ => Value::Null,
        }
    }
    #[cfg(windows)]
    pub fn on_plugin_refresh(args: &[Value]) -> Value {
        let cb = args.first().filter(|v| matches!(v, Value::Function(_))).cloned();
        crate::pluginhost::on_refresh(cb);
        Value::Null
    }
    #[cfg(not(windows))]
    pub fn on_plugin_refresh(_a: &[Value]) -> Value { Value::Null }
}

pub fn moo_object() -> Value {
    let mut m = ObjectMap::default();
    m.insert(Arc::from("reindex"), Value::native(native_reindex));
    m.insert(Arc::from("search"), Value::native(native_search));
    m.insert(Arc::from("setup"), Value::native(natives::setup));
    m.insert(Arc::from("setPanelShape"), Value::native(natives::set_panel_shape));
    m.insert(Arc::from("setArrowKeys"), Value::native(natives::set_arrow_keys));
    m.insert(Arc::from("typeText"), Value::native(natives::type_text));
    m.insert(Arc::from("setTheme"), Value::native(natives::set_theme));
    m.insert(Arc::from("registerHotkey"), Value::native(natives::register_hotkey));
    m.insert(Arc::from("unregisterHotkey"), Value::native(natives::unregister_hotkey));
    m.insert(Arc::from("checkHotkey"), Value::native(natives::check_hotkey));
    m.insert(Arc::from("hotkeyDisplay"), Value::native(natives::hotkey_display));
    m.insert(Arc::from("recordHotkey"), Value::native(natives::record_hotkey));
    m.insert(Arc::from("toggle"), Value::native(natives::toggle));
    m.insert(Arc::from("setPanelKeys"), Value::native(natives::set_panel_keys));
    m.insert(Arc::from("isRightClick"), Value::native(natives::is_right_click));
    m.insert(Arc::from("keysError"), Value::native(natives::keys_error));
    m.insert(Arc::from("localDateTime"), Value::native(natives::local_date_time));
    m.insert(Arc::from("cliMain"), Value::native(natives::cli_main));
    m.insert(Arc::from("cliServe"), Value::native(natives::cli_serve));
    m.insert(Arc::from("cliWrite"), Value::native(natives::cli_write));
    m.insert(Arc::from("cliEnd"), Value::native(natives::cli_end));
    m.insert(Arc::from("show"), Value::native(natives::show));
    m.insert(Arc::from("hide"), Value::native(natives::hide));
    m.insert(Arc::from("quit"), Value::native(natives::quit));
    m.insert(Arc::from("appCount"), Value::native(native_app_count));
    m.insert(Arc::from("appFolders"), Value::native(native_app_folders));
    m.insert(Arc::from("appPaths"), Value::native(native_app_paths));
    m.insert(Arc::from("fuzzy"), Value::native(native_fuzzy));
    m.insert(Arc::from("pluginPaths"), Value::native(native_plugin_paths));
    m.insert(Arc::from("loadBytecodePlugin"), Value::native(native_load_bytecode_plugin));
    m.insert(Arc::from("fileIndexStart"), Value::native(natives::file_index_start));
    m.insert(Arc::from("findFiles"), Value::native(natives::find_files));
    m.insert(Arc::from("recentFiles"), Value::native(natives::recent_files));
    m.insert(Arc::from("watchTyped"), Value::native(natives::watch_typed));
    m.insert(Arc::from("quickLook"), Value::native(natives::quick_look));
    m.insert(Arc::from("quickLookVisible"), Value::native(natives::quick_look_visible));
    m.insert(Arc::from("fileIndexStatus"), Value::native(natives::file_index_status));
    m.insert(Arc::from("bundleResources"), Value::native(native_bundle_resources));
    m.insert(Arc::from("exeDir"), Value::native(native_exe_dir));
    m.insert(Arc::from("about"), Value::native(native_about));
    m.insert(Arc::from("recordUse"), Value::native(native_record_use));
    m.insert(Arc::from("frecency"), Value::native(native_frecency));
    m.insert(Arc::from("aiAvailability"), Value::native(natives::ai_availability));
    m.insert(Arc::from("aiSession"), Value::native(natives::ai_session));
    m.insert(Arc::from("aiEndSession"), Value::native(natives::ai_end_session));
    m.insert(Arc::from("aiPrewarm"), Value::native(natives::ai_prewarm));
    m.insert(Arc::from("aiAsk"), Value::native(natives::ai_ask));
    m.insert(Arc::from("aiCancel"), Value::native(natives::ai_cancel));
    m.insert(Arc::from("keychainGet"), Value::native(natives::keychain_get));
    m.insert(Arc::from("keychainSet"), Value::native(natives::keychain_set));
    m.insert(Arc::from("keychainDelete"), Value::native(natives::keychain_delete));
    m.insert(Arc::from("keychainHas"), Value::native(natives::keychain_has));
    m.insert(Arc::from("oauthSignIn"), Value::native(natives::oauth_sign_in));
    m.insert(Arc::from("oauthCancel"), Value::native(natives::oauth_cancel));
    m.insert(Arc::from("oauthRefresh"), Value::native(natives::oauth_refresh));
    m.insert(Arc::from("withUi"), Value::native(natives::with_ui));
    m.insert(Arc::from("onPluginRefresh"), Value::native(natives::on_plugin_refresh));
    Value::object(m)
}
