//! `nimble-macos`: native launcher services for the Nimble shell, imported from Tish as
//! `import { reindex, search, launch, setup, ... } from "nimble-macos"`.

#[cfg(target_os = "macos")]
mod ai;
#[cfg(target_os = "macos")]
mod clip;
#[cfg(target_os = "macos")]
mod files;
mod frecency;
mod index;
#[cfg(target_os = "macos")]
mod keymap;
#[cfg(target_os = "macos")]
mod mac;
mod vmplug;
#[cfg(target_os = "macos")]
mod watch;

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

fn obj(pairs: Vec<(&str, Value)>) -> Value {
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

/// `bundleResources()` -> `Nimble.app/Contents/Resources` when running from an app bundle, else null.
fn native_bundle_resources(_args: &[Value]) -> Value {
    let exe = std::env::current_exe().ok().and_then(|p| p.canonicalize().ok());
    let contents = exe.as_deref().and_then(|p| p.parent()).filter(|d| d.ends_with("MacOS")).and_then(|d| d.parent());
    match contents {
        Some(c) if c.ends_with("Contents") && c.parent().is_some_and(|b| b.extension().is_some_and(|e| e == "app")) => {
            Value::String(c.join("Resources").to_string_lossy().as_ref().into())
        }
        _ => Value::Null,
    }
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

fn native_app_count(_args: &[Value]) -> Value {
    Value::Number(index::app_count() as f64)
}

fn native_search(args: &[Value]) -> Value {
    let limit = num_arg(args, 1, 8.0).max(0.0) as usize;
    let rows: Vec<Value> = index::search(&str_arg(args, 0), limit)
        .into_iter()
        .map(|(app, score)| {
            #[cfg(target_os = "macos")]
            let icon = mac::icon_name(&app.path);
            #[cfg(not(target_os = "macos"))]
            let icon = String::new();
            obj(vec![
                ("name", Value::String(app.name.as_str().into())),
                ("path", Value::String(app.path.as_str().into())),
                ("icon", Value::String(icon.as_str().into())),
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

    /// `launch(target)`: open a file path or a URL (`scheme://...`); hides Nimble on success.
    pub fn launch(args: &[Value]) -> Value {
        let ok = mac::launch(&str_arg(args, 0));
        if ok {
            mac::hide();
        }
        Value::Bool(ok)
    }

    pub fn copy_text(args: &[Value]) -> Value {
        Value::Bool(mac::copy_text(&str_arg(args, 0)))
    }

    /// `setup({ width, height, onKey, onShow })`: call before `macos.run(App)`.
    pub fn setup(args: &[Value]) -> Value {
        let opts = args.first();
        let w = field(opts, "width").and_then(|v| v.as_number()).unwrap_or(720.0);
        let h = field(opts, "height").and_then(|v| v.as_number()).unwrap_or(440.0);
        mac::set_panel_size(w, h);
        mac::set_callbacks(field(opts, "onKey"), field(opts, "onShow"));
        mac::schedule_setup();
        Value::Null
    }

    /// `registerHotkey(spec)` -> `{ ok, display, registered }` or `{ ok: false, error }`. `spec` names
    /// keys as printed ("cmd+space"); `registered` lists what was registered after the keyboards'
    /// modifier mappings (["ctrl+space"] with Command and Control swapped); `display` is "⌘Space".
    pub fn register_hotkey(args: &[Value]) -> Value {
        match mac::register_hotkey(&str_arg(args, 0)) {
            Ok(h) => obj(vec![
                ("ok", Value::Bool(true)),
                ("display", Value::String(h.display.as_str().into())),
                (
                    "registered",
                    Value::Array(VmRef::new(h.registered.iter().map(|s| Value::String(s.as_str().into())).collect())),
                ),
            ]),
            Err(e) => obj(vec![("ok", Value::Bool(false)), ("error", Value::String(e.as_str().into()))]),
        }
    }

    pub fn show(_a: &[Value]) -> Value {
        mac::show();
        Value::Null
    }

    pub fn hide(_a: &[Value]) -> Value {
        mac::hide();
        Value::Null
    }

    pub fn quit(_a: &[Value]) -> Value {
        mac::quit();
        Value::Null
    }

    /// `searchFiles(query, limit, onResults)`: Spotlight-backed, asynchronous. `onResults` gets
    /// `{ query, results, ms }` on the main thread, only for the newest query.
    pub fn search_files(args: &[Value]) -> Value {
        let limit = num_arg(args, 1, 8.0).max(0.0) as usize;
        Value::Number(mac::search_files(&str_arg(args, 0), limit, args.get(2).cloned()) as f64)
    }

    /// `watchApps()`: keep the app index live with FSEvents on the application folders.
    pub fn watch_apps(_a: &[Value]) -> Value {
        Value::Bool(mac::watch_apps())
    }

    /// `statusItem(hotkey)`: menu bar icon with Show / Quit.
    pub fn status_item(args: &[Value]) -> Value {
        Value::Bool(mac::status_item(&str_arg(args, 0)))
    }

    /// `watchClipboard()`: start recording text clipboard history (in memory only).
    pub fn watch_clipboard(_a: &[Value]) -> Value {
        Value::Bool(clip::watch())
    }

    /// `clipboardHistory(query, limit)` -> `[{ text, preview, app, icon, ago }]`, newest first.
    pub fn clipboard_history(args: &[Value]) -> Value {
        let limit = num_arg(args, 1, 8.0).max(0.0) as usize;
        let rows: Vec<Value> = clip::history(&str_arg(args, 0), limit)
            .into_iter()
            .map(|c| {
                let icon = if c.app_path.is_empty() { String::new() } else { mac::icon_name(&c.app_path) };
                obj(vec![
                    ("text", Value::String(c.text.as_str().into())),
                    ("preview", Value::String(clip::preview(&c.text, 80).as_str().into())),
                    ("app", Value::String(c.app.as_str().into())),
                    ("icon", Value::String(icon.as_str().into())),
                    ("ago", Value::String(clip::ago(c.at).as_str().into())),
                ])
            })
            .collect();
        Value::Array(VmRef::new(rows))
    }

    pub fn clear_clipboard_history(_a: &[Value]) -> Value {
        clip::clear();
        Value::Null
    }

    /// `aiAvailability()` -> `"available"`, or why Apple's on-device model cannot answer.
    pub fn ai_availability(_a: &[Value]) -> Value {
        match ai::availability() {
            Ok(()) => Value::String("available".into()),
            Err(reason) => Value::String(reason.as_str().into()),
        }
    }

    /// `aiSession(instructions)` -> session id (0 when unavailable). Asks in a session share context.
    pub fn ai_session(args: &[Value]) -> Value {
        Value::Number(ai::session(&str_arg(args, 0)) as f64)
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

    /// `symbolIcon(name)` -> an image name for `<image src>` showing SF Symbol `name`.
    pub fn symbol_icon(args: &[Value]) -> Value {
        Value::String(mac::symbol_icon(&str_arg(args, 0)).as_str().into())
    }
}

#[cfg(not(target_os = "macos"))]
mod natives {
    use super::*;
    pub fn launch(_a: &[Value]) -> Value { Value::Bool(false) }
    pub fn copy_text(_a: &[Value]) -> Value { Value::Bool(false) }
    pub fn setup(_a: &[Value]) -> Value { Value::Null }
    pub fn register_hotkey(_a: &[Value]) -> Value {
        obj(vec![("ok", Value::Bool(false)), ("error", Value::String("macOS only".into()))])
    }
    pub fn show(_a: &[Value]) -> Value { Value::Null }
    pub fn hide(_a: &[Value]) -> Value { Value::Null }
    pub fn quit(_a: &[Value]) -> Value { Value::Null }
    pub fn search_files(_a: &[Value]) -> Value { Value::Number(0.0) }
    pub fn watch_apps(_a: &[Value]) -> Value { Value::Bool(false) }
    pub fn status_item(_a: &[Value]) -> Value { Value::Bool(false) }
    pub fn watch_clipboard(_a: &[Value]) -> Value { Value::Bool(false) }
    pub fn clipboard_history(_a: &[Value]) -> Value { Value::Array(VmRef::new(Vec::new())) }
    pub fn clear_clipboard_history(_a: &[Value]) -> Value { Value::Null }
    pub fn ai_availability(_a: &[Value]) -> Value { Value::String("macOS only".into()) }
    pub fn ai_session(_a: &[Value]) -> Value { Value::Number(0.0) }
    pub fn ai_end_session(_a: &[Value]) -> Value { Value::Null }
    pub fn ai_prewarm(_a: &[Value]) -> Value { Value::Null }
    pub fn ai_ask(_a: &[Value]) -> Value { Value::Number(0.0) }
    pub fn ai_cancel(_a: &[Value]) -> Value { Value::Null }
    pub fn symbol_icon(_a: &[Value]) -> Value { Value::String("".into()) }
}

pub fn nimble_object() -> Value {
    let mut m = ObjectMap::default();
    m.insert(Arc::from("reindex"), Value::native(native_reindex));
    m.insert(Arc::from("search"), Value::native(native_search));
    m.insert(Arc::from("launch"), Value::native(natives::launch));
    m.insert(Arc::from("setup"), Value::native(natives::setup));
    m.insert(Arc::from("registerHotkey"), Value::native(natives::register_hotkey));
    m.insert(Arc::from("show"), Value::native(natives::show));
    m.insert(Arc::from("hide"), Value::native(natives::hide));
    m.insert(Arc::from("quit"), Value::native(natives::quit));
    m.insert(Arc::from("appCount"), Value::native(native_app_count));
    m.insert(Arc::from("fuzzy"), Value::native(native_fuzzy));
    m.insert(Arc::from("pluginPaths"), Value::native(native_plugin_paths));
    m.insert(Arc::from("copyText"), Value::native(natives::copy_text));
    m.insert(Arc::from("loadBytecodePlugin"), Value::native(native_load_bytecode_plugin));
    m.insert(Arc::from("searchFiles"), Value::native(natives::search_files));
    m.insert(Arc::from("watchApps"), Value::native(natives::watch_apps));
    m.insert(Arc::from("bundleResources"), Value::native(native_bundle_resources));
    m.insert(Arc::from("recordUse"), Value::native(native_record_use));
    m.insert(Arc::from("frecency"), Value::native(native_frecency));
    m.insert(Arc::from("statusItem"), Value::native(natives::status_item));
    m.insert(Arc::from("watchClipboard"), Value::native(natives::watch_clipboard));
    m.insert(Arc::from("clipboardHistory"), Value::native(natives::clipboard_history));
    m.insert(Arc::from("clearClipboardHistory"), Value::native(natives::clear_clipboard_history));
    m.insert(Arc::from("aiAvailability"), Value::native(natives::ai_availability));
    m.insert(Arc::from("aiSession"), Value::native(natives::ai_session));
    m.insert(Arc::from("aiEndSession"), Value::native(natives::ai_end_session));
    m.insert(Arc::from("aiPrewarm"), Value::native(natives::ai_prewarm));
    m.insert(Arc::from("aiAsk"), Value::native(natives::ai_ask));
    m.insert(Arc::from("aiCancel"), Value::native(natives::ai_cancel));
    m.insert(Arc::from("symbolIcon"), Value::native(natives::symbol_icon));
    Value::object(m)
}
