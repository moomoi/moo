//! `moo-macos`: native launcher services for the Moo shell, imported from Tish as
//! `import { reindex, search, launch, setup, ... } from "moo-macos"`.

#[cfg(target_os = "macos")]
mod ai;
#[cfg(target_os = "macos")]
mod ax;
#[cfg(target_os = "macos")]
mod bridge;
mod calc;
#[cfg(target_os = "macos")]
mod chats;
#[cfg(target_os = "macos")]
mod cli;
#[cfg(target_os = "macos")]
mod clip;
#[cfg(target_os = "macos")]
mod contacts;
#[cfg(target_os = "macos")]
mod dict;
#[cfg(target_os = "macos")]
mod fileops;
#[cfg(target_os = "macos")]
mod files;
mod frecency;
mod fsindex;
#[cfg(target_os = "macos")]
mod fslive;
mod history;
#[cfg(target_os = "macos")]
mod http;
mod prefs;
mod index;
#[cfg(target_os = "macos")]
mod keychain;
#[cfg(target_os = "macos")]
mod keymap;
#[cfg(target_os = "macos")]
mod keys;
mod layout;
#[cfg(target_os = "macos")]
mod mac;
#[cfg(target_os = "macos")]
mod oauth;
#[cfg(target_os = "macos")]
mod pluginhost;
#[cfg(target_os = "macos")]
mod rates;
#[cfg(target_os = "macos")]
mod remote;
#[cfg(target_os = "macos")]
mod shell;
#[cfg(target_os = "macos")]
mod siteicon;
#[cfg(unix)]
mod shortcuts;
mod snippets;
#[cfg(target_os = "macos")]
mod sysinfo;
#[cfg(target_os = "macos")]
mod system;
#[cfg(target_os = "macos")]
mod theme;
#[cfg(target_os = "macos")]
mod tz;
mod vmplug;
#[cfg(target_os = "macos")]
mod watch;
mod websearch;

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
    #[cfg(target_os = "macos")]
    mac::warm_app_icons();
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

/// `searchHistory(limit)` -> recent queries, newest first.
fn native_search_history(args: &[Value]) -> Value {
    let limit = num_arg(args, 0, 20.0).max(0.0) as usize;
    Value::Array(VmRef::new(history::list(limit).into_iter().map(|q| Value::String(q.as_str().into())).collect()))
}

/// `addSearchHistory(query)`: move `query` to the top of the search history.
fn native_add_search_history(args: &[Value]) -> Value {
    history::add(&str_arg(args, 0));
    Value::Null
}

fn native_clear_search_history(_args: &[Value]) -> Value {
    history::clear();
    Value::Null
}

/// `getPref(key)` -> the remembered value, or "".
fn native_get_pref(args: &[Value]) -> Value {
    Value::String(prefs::get(&str_arg(args, 0)).as_str().into())
}

/// `setPref(key, value)`: remember `value` between runs.
fn native_set_pref(args: &[Value]) -> Value {
    prefs::set(&str_arg(args, 0), &str_arg(args, 1));
    Value::Null
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
    use dispatch2::DispatchQueue;

    /// `launch(target)`: open a file path or a URL (`scheme://...`); hides Moo on success.
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

    fn config_value(cfg: &shortcuts::Config, warnings: &[String], error: &str) -> Value {
        let path = shortcuts::config_path().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        let list = cfg
            .shortcuts
            .iter()
            .map(|x| {
                obj(vec![
                    ("keyword", s(&x.keyword)),
                    ("name", s(&x.name)),
                    ("kind", s(x.kind.name())),
                    ("target", s(&x.target)),
                    ("input", s(&x.input)),
                    ("output", s(&x.output)),
                    ("hotkey", s(&x.hotkey)),
                    ("expand", Value::Bool(x.kind == shortcuts::Kind::Text && x.expand)),
                    ("needsQuery", Value::Bool(shortcuts::needs_query(if x.kind == shortcuts::Kind::Command { &x.input } else { &x.target }))),
                ])
            })
            .collect();
        let hotkeys = cfg
            .hotkeys
            .iter()
            .map(|h| obj(vec![("keys", s(&h.keys)), ("run", s(&h.run)), ("query", s(&h.query))]))
            .collect();
        obj(vec![
            ("ok", Value::Bool(error.is_empty())),
            ("error", s(error)),
            ("path", s(&path)),
            ("launcher", s(&cfg.launcher)),
            ("aiModel", s(&cfg.ai.model)),
            ("shortcuts", arr(list)),
            ("hotkeys", arr(hotkeys)),
            ("warnings", arr(warnings.iter().map(|w| s(w)).collect())),
        ])
    }

    /// `loadShortcuts()` -> `{ ok, error, path, launcher, aiModel, shortcuts, hotkeys, warnings }`.
    pub fn load_shortcuts(_a: &[Value]) -> Value {
        let Some(path) = shortcuts::config_path() else { return config_value(&Default::default(), &[], "HOME is not set") };
        match shortcuts::load(&path) {
            Ok((cfg, w)) => config_value(&cfg, &w, ""),
            Err(e) => config_value(&Default::default(), &[], &e),
        }
    }

    fn edit_config(f: impl FnOnce(&mut shortcuts::Config) -> Result<(), String>) -> Value {
        let result = (|| {
            let path = shortcuts::config_path().ok_or("HOME is not set")?;
            let (mut cfg, _) = shortcuts::load(&path)?;
            f(&mut cfg)?;
            shortcuts::save(&path, &cfg)
        })();
        match result {
            Ok(()) => obj(vec![("ok", Value::Bool(true))]),
            Err(e) => obj(vec![("ok", Value::Bool(false)), ("error", s(&e))]),
        }
    }

    fn text_field(v: Option<&Value>, key: &str) -> String {
        match field(v, key) {
            Some(Value::String(x)) => x.to_string(),
            _ => String::new(),
        }
    }

    /// `saveShortcut({ keyword, name, kind, target, input?, output?, hotkey? }, replacing?)` ->
    /// `{ ok, error }`. `replacing` is the keyword of the shortcut being edited.
    pub fn save_shortcut(args: &[Value]) -> Value {
        let v = args.first();
        let kind_name = text_field(v, "kind");
        let Some(kind) = shortcuts::Kind::parse(&kind_name) else {
            return obj(vec![("ok", Value::Bool(false)), ("error", s(&format!("unknown kind `{kind_name}` (url, open, command, shell, text)")))]);
        };
        let hotkey = text_field(v, "hotkey");
        if !hotkey.is_empty() {
            if let Err(e) = keys::parse(&hotkey) {
                return obj(vec![("ok", Value::Bool(false)), ("error", s(&e))]);
            }
        }
        let sc = shortcuts::Shortcut {
            keyword: text_field(v, "keyword"),
            name: text_field(v, "name"),
            kind,
            target: text_field(v, "target"),
            input: text_field(v, "input"),
            output: text_field(v, "output"),
            hotkey,
            model: text_field(v, "model"),
            expand: matches!(field(v, "expand"), Some(Value::Bool(true))),
        };
        let replacing = match args.get(1) {
            Some(Value::String(r)) if !r.is_empty() => Some(r.to_string()),
            _ => None,
        };
        edit_config(|cfg| shortcuts::upsert(cfg, sc, replacing.as_deref()))
    }

    /// `ensureShortcutsFile()` -> its path, creating an empty one if missing (never rewrites).
    pub fn ensure_shortcuts_file(_a: &[Value]) -> Value {
        let Some(path) = shortcuts::config_path() else { return s("") };
        if !path.exists() {
            if let Err(e) = shortcuts::save(&path, &Default::default()) {
                eprintln!("moo: {e}");
            }
        }
        s(&path.to_string_lossy())
    }

    /// `checkKeyword(keyword, replacing?)` -> `{ ok, error }`.
    pub fn check_keyword(args: &[Value]) -> Value {
        let cfg = shortcuts::config_path().and_then(|p| shortcuts::load(&p).ok()).map(|(c, _)| c).unwrap_or_default();
        let replacing = str_arg(args, 1);
        match shortcuts::check_keyword(&cfg, &str_arg(args, 0), Some(replacing.as_str()).filter(|r| !r.is_empty())) {
            Ok(()) => obj(vec![("ok", Value::Bool(true))]),
            Err(e) => obj(vec![("ok", Value::Bool(false)), ("error", s(&e))]),
        }
    }

    pub fn remove_shortcut(args: &[Value]) -> Value {
        let kw = str_arg(args, 0);
        edit_config(|cfg| if shortcuts::remove(cfg, &kw) { Ok(()) } else { Err(format!("no shortcut `{kw}`")) })
    }

    /// `bindHotkey(keys, run, query?)`: add or replace a hotkey binding in the file.
    pub fn bind_hotkey(args: &[Value]) -> Value {
        let (keys_spec, run, query) = (str_arg(args, 0), str_arg(args, 1), str_arg(args, 2));
        if let Err(e) = keys::parse(&keys_spec) {
            return obj(vec![("ok", Value::Bool(false)), ("error", s(&e))]);
        }
        edit_config(|cfg| {
            shortcuts::bind(cfg, &keys_spec, &run, &query);
            Ok(())
        })
    }

    pub fn unbind_hotkey(args: &[Value]) -> Value {
        let k = str_arg(args, 0);
        edit_config(|cfg| if shortcuts::unbind(cfg, &k) { Ok(()) } else { Err(format!("no hotkey `{k}`")) })
    }

    /// `setLauncherHotkey(keys)`: save the launcher hotkey as `"launcher"` in the file ("" removes
    /// it). Registering it is up to the caller.
    pub fn set_launcher_hotkey(args: &[Value]) -> Value {
        let keys_spec = str_arg(args, 0);
        if !keys_spec.is_empty() {
            if let Err(e) = keys::parse(&keys_spec) {
                return obj(vec![("ok", Value::Bool(false)), ("error", s(&e))]);
            }
        }
        edit_config(|cfg| {
            cfg.launcher = keys_spec;
            Ok(())
        })
    }

    /// `expandTemplate(template, query, kind)`: fill `{query}` (encoded for the kind),
    /// `{clipboard}`, `{date}` and `{time}`; `~/` at the start becomes the home folder for `open`.
    pub fn expand_template(args: &[Value]) -> Value {
        let template = str_arg(args, 0);
        let kind = shortcuts::Kind::parse(&str_arg(args, 2)).unwrap_or(shortcuts::Kind::Text);
        let mut vars = shortcuts::Vars { query: str_arg(args, 1), ..Default::default() };
        if template.contains("{clipboard}") {
            vars.clipboard = mac::clipboard_text();
        }
        if template.contains("{selection}") {
            vars.selection = crate::ax::selected_text().unwrap_or_default();
        }
        if template.contains("{date}") || template.contains("{time}") {
            (vars.date, vars.time) = shortcuts::local_date_time();
        }
        let mut out = shortcuts::expand(&template, &vars, shortcuts::Encoding::for_kind(kind));
        if kind == shortcuts::Kind::Open && (out == "~" || out.starts_with("~/")) {
            if let Some(home) = std::env::var_os("HOME") {
                out = format!("{}{}", home.to_string_lossy(), &out[1..]);
            }
        }
        s(&out)
    }

    thread_local! {
        static ON_CONFIG: std::cell::RefCell<Option<Value>> = const { std::cell::RefCell::new(None) };
    }

    fn config_changed() {
        let Some(Value::Function(f)) = ON_CONFIG.with(|c| c.borrow().clone()) else { return };
        crate::mac::with_ui(|| {
            let _ = f.call(&[]);
        });
    }

    /// `watchShortcuts(cb)`: call `cb()` when shortcuts.json changes (edited by hand, by the CLI or
    /// by Moo itself). Creates the folder so it can be watched.
    pub fn watch_shortcuts(args: &[Value]) -> Value {
        let Some(dir) = shortcuts::config_path().and_then(|p| p.parent().map(|d| d.to_path_buf())) else { return Value::Bool(false) };
        let _ = std::fs::create_dir_all(&dir);
        ON_CONFIG.with(|c| *c.borrow_mut() = args.first().cloned());
        Value::Bool(watch::watch_with_latency(&[dir.to_string_lossy().into_owned()], config_changed, 0.3))
    }

    fn shell_value(id: u64, o: shell::Output) -> Value {
        obj(vec![
            ("id", Value::Number(id as f64)),
            ("code", Value::Number(o.code as f64)),
            ("stdout", s(&o.stdout)),
            ("stderr", s(&o.stderr)),
            ("ms", Value::Number(o.ms)),
            ("timedOut", Value::Bool(o.timed_out)),
        ])
    }

    /// `runShell(command, cwd, cb)` -> id; `cb({ id, code, stdout, stderr, ms, timedOut })` later.
    pub fn run_shell(args: &[Value]) -> Value {
        let cb = args.get(2).cloned().unwrap_or(Value::Null);
        Value::Number(shell::run(&str_arg(args, 0), &str_arg(args, 1), cb, shell_value) as f64)
    }

    pub fn clipboard_text(_a: &[Value]) -> Value {
        s(&mac::clipboard_text())
    }

    // ── Command line ──

    /// `cliMain()`: when this process was started as a command (`moo files foo`), or another
    /// Moo is already running, act as its client and exit. Otherwise return false: be the app.
    pub fn cli_main(_a: &[Value]) -> Value {
        let a = cli::args();
        if !a.is_empty() || cli::running() {
            std::process::exit(cli::client(&a));
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

    pub fn cli_usage(_a: &[Value]) -> Value {
        s(cli::USAGE)
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

    /// `searchFiles(query, limit, onResults)`: Spotlight-backed, asynchronous. `onResults` gets
    /// `{ query, results, ms }` on the main thread, only for the newest query.
    pub fn search_files(args: &[Value]) -> Value {
        let limit = num_arg(args, 1, 8.0).max(0.0) as usize;
        Value::Number(mac::search_files(&str_arg(args, 0), limit, args.get(2).cloned()) as f64)
    }

    /// `fileIndexStart()`: build the file index in the background (snapshot or crawl) and keep it
    /// live. Returns at once.
    pub fn file_index_start(_args: &[Value]) -> Value {
        Value::Bool(fslive::start())
    }

    fn file_rows(hits: Vec<fsindex::FileHit>, icons: usize) -> Value {
        let rows: Vec<Value> = hits
            .into_iter()
            .enumerate()
            .map(|(i, h)| {
                let icon = if i < icons { mac::icon_name(&h.path) } else { String::new() };
                obj(vec![
                    ("name", Value::String(h.name.as_str().into())),
                    ("path", Value::String(h.path.as_str().into())),
                    ("icon", Value::String(icon.as_str().into())),
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
        let icons = num_arg(args, 2, 8.0).max(0.0) as usize;
        match fslive::search(&str_arg(args, 0), limit) {
            Some((hits, ms)) => obj(vec![
                ("ready", Value::Bool(true)),
                ("results", file_rows(hits, icons)),
                ("ms", Value::Number(ms)),
            ]),
            None => obj(vec![("ready", Value::Bool(false)), ("results", Value::Array(VmRef::new(vec![]))), ("ms", Value::Number(0.0))]),
        }
    }

    /// `recentFiles(limit)`: files and folders opened through Moo, most used first.
    pub fn recent_files(args: &[Value]) -> Value {
        let limit = num_arg(args, 0, 8.0).max(0.0) as usize;
        file_rows(fslive::recent(limit), limit)
    }

    /// `queryFiles({ name, kind, minBytes, maxBytes, createdSecs, modifiedSecs, openedSecs, folder,
    /// sort, limit })` -> `{ results: [{ name, path, icon, kind, detail, bytes, created, modified,
    /// opened }], total, ms }`. Spotlight metadata search, synchronous; dates are Unix ms (0: none).
    pub fn query_files(args: &[Value]) -> Value {
        let f = args.first();
        let text = |k| match field(f, k) {
            Some(Value::String(s)) => s.to_string(),
            _ => String::new(),
        };
        let num = |k| field(f, k).and_then(|v| v.as_number()).unwrap_or(0.0).max(0.0);
        let filter = files::Filter {
            name: text("name"),
            contains: text("contains"),
            kind: text("kind"),
            min_bytes: num("minBytes") as u64,
            max_bytes: num("maxBytes") as u64,
            created_secs: num("createdSecs"),
            modified_secs: num("modifiedSecs"),
            opened_secs: num("openedSecs"),
            folder: text("folder"),
            sort: text("sort"),
            limit: (num("limit") as usize).clamp(1, 50),
        };
        let r = files::find(&filter);
        let ms = |t: f64| Value::Number((t * 1000.0).round());
        let rows: Vec<Value> = r
            .hits
            .into_iter()
            .map(|h| {
                obj(vec![
                    ("name", Value::String(h.name.as_str().into())),
                    ("icon", Value::String(mac::icon_name(&h.path).as_str().into())),
                    ("kind", Value::String(if h.is_dir { "Folder" } else { "File" }.into())),
                    ("detail", Value::String(h.detail.as_str().into())),
                    ("bytes", Value::Number(h.bytes as f64)),
                    ("created", ms(h.created)),
                    ("modified", ms(h.modified)),
                    ("opened", ms(h.opened)),
                    ("path", Value::String(h.path.as_str().into())),
                ])
            })
            .collect();
        obj(vec![
            ("results", Value::Array(VmRef::new(rows))),
            ("total", Value::Number(r.total as f64)),
            ("ms", Value::Number(r.ms)),
        ])
    }

    /// `calculate(text)` -> `{ display, copy, detail }` or null: arithmetic, units, currency (ECB
    /// rates, refreshed in the background when over 12 h old), number bases and time zones.
    pub fn calculate(args: &[Value]) -> Value {
        rates::refresh_if_stale();
        let text = str_arg(args, 0);
        let rates = rates::get().map(|(r, _)| r);
        match calc::answer(&text, rates.as_ref()).or_else(|| tz::answer(&text)) {
            Some(a) => obj(vec![
                ("display", Value::String(a.display.as_str().into())),
                ("copy", Value::String(a.copy.as_str().into())),
                ("detail", Value::String(a.detail.as_str().into())),
            ]),
            None => Value::Null,
        }
    }

    /// `define(word)` -> `{ word, pronunciation, senses: [{ part, pronunciation, definition, example }],
    /// origin }` or null: every homograph's main senses from the system dictionary.
    pub fn define(args: &[Value]) -> Value {
        match crate::dict::define(&str_arg(args, 0)) {
            Some(e) => obj(vec![
                ("word", s(&e.word)),
                ("pronunciation", s(&e.pronunciation)),
                (
                    "senses",
                    arr(e
                        .senses
                        .iter()
                        .map(|x| {
                            obj(vec![
                                ("part", s(&x.part)),
                                ("pronunciation", s(&x.pronunciation)),
                                ("definition", s(&x.definition)),
                                ("example", s(&x.example)),
                            ])
                        })
                        .collect()),
                ),
                ("origin", s(&e.origin)),
            ]),
            None => Value::Null,
        }
    }

    /// `dictionaryWarm()`: loads the dictionary on a background thread, so the first lookup while
    /// typing does not wait for it (about 60 ms cold).
    pub fn dictionary_warm(_a: &[Value]) -> Value {
        std::thread::spawn(|| {
            crate::dict::define("a");
        });
        Value::Null
    }

    /// `runningApps()` -> `[{ name, path, icon, pid, bundleId, active, hidden, memory, memoryText }]`:
    /// apps with a Dock icon, Moo left out, most memory first.
    pub fn running_apps(_a: &[Value]) -> Value {
        let rows: Vec<Value> = crate::system::running_apps()
            .into_iter()
            .map(|a| {
                obj(vec![
                    ("name", s(&a.name)),
                    ("icon", s(&mac::icon_name(&a.path))),
                    ("path", s(&a.path)),
                    ("pid", Value::Number(a.pid as f64)),
                    ("bundleId", s(&a.bundle_id)),
                    ("active", Value::Bool(a.active)),
                    ("hidden", Value::Bool(a.hidden)),
                    ("memory", Value::Number(a.memory as f64)),
                    ("memoryText", s(&crate::system::bytes(a.memory))),
                ])
            })
            .collect();
        Value::Array(VmRef::new(rows))
    }

    /// `appAction(pid, action)` -> `{ ok, message }`; action is switch, hide, unhide, quit or
    /// force-quit.
    pub fn app_action(args: &[Value]) -> Value {
        let action = str_arg(args, 1);
        let (ok, message) = match crate::system::app_action(num_arg(args, 0, 0.0) as i32, &action) {
            Ok(m) => (true, m),
            Err(e) => (false, e),
        };
        obj(vec![("ok", Value::Bool(ok)), ("message", s(&message))])
    }

    /// `systemCommand(id, arg, cb)`: lock, sleep, sleep-displays, restart, shut-down, log-out,
    /// empty-trash, screen-saver, dark-mode, mute, volume, eject, quit-all, hide-all.
    /// `cb({ ok, message })` runs later on the main thread; without `cb` the result is returned.
    pub fn system_command(args: &[Value]) -> Value {
        fn result(r: Result<String, String>) -> Value {
            let (ok, message) = match r {
                Ok(m) => (true, m),
                Err(e) => (false, e),
            };
            obj(vec![("ok", Value::Bool(ok)), ("message", s(&message))])
        }
        if !matches!(args.get(2), Some(Value::Function(_))) {
            return result(crate::system::run(&str_arg(args, 0), &str_arg(args, 1)));
        }
        thread_local! {
            static CALLBACKS: std::cell::RefCell<std::collections::HashMap<u64, Value>> = Default::default();
        }
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        fn reply(id: u64, r: Result<String, String>) {
            DispatchQueue::main().exec_async(move || {
                let Some(Value::Function(f)) = CALLBACKS.with(|c| c.borrow_mut().remove(&id)) else { return };
                crate::mac::with_ui(|| {
                    let _ = f.call(&[result(r)]);
                });
            });
        }
        let (cmd, arg) = (str_arg(args, 0), str_arg(args, 1));
        let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        CALLBACKS.with(|c| c.borrow_mut().insert(id, args.get(2).cloned().unwrap_or(Value::Null)));
        if crate::system::blocks(&cmd) {
            std::thread::spawn(move || reply(id, crate::system::run(&cmd, &arg)));
        } else {
            reply(id, crate::system::run(&cmd, &arg));
        }
        Value::Null
    }

    /// `arrangeWindow(layout)` -> `{ ok, message }`: move the frontmost app's focused window
    /// (left-half, right-third, maximize, next-display, restore, …).
    pub fn arrange_window(args: &[Value]) -> Value {
        let (ok, message) = match crate::ax::arrange(&str_arg(args, 0)) {
            Ok(m) => (true, m),
            Err(e) => (false, e),
        };
        obj(vec![("ok", Value::Bool(ok)), ("message", s(&message))])
    }

    fn result_value(r: Result<String, String>) -> Value {
        match r {
            Ok(m) => obj(vec![("ok", Value::Bool(true)), ("message", s(&m))]),
            Err(e) => obj(vec![("ok", Value::Bool(false)), ("error", s(&e))]),
        }
    }

    /// `trashFile(path)` -> `{ ok, message, error }`; message is where the file went.
    pub fn trash_file(args: &[Value]) -> Value {
        result_value(crate::fileops::trash(&str_arg(args, 0)))
    }

    /// `appsFor(path)` -> `[{ name, path, icon, isDefault }]`, the default app first.
    pub fn apps_for(args: &[Value]) -> Value {
        let rows = crate::fileops::apps_for(&str_arg(args, 0))
            .into_iter()
            .map(|a| obj(vec![("name", s(&a.name)), ("icon", s(&mac::icon_name(&a.path))), ("path", s(&a.path)), ("isDefault", Value::Bool(a.default))]))
            .collect();
        arr(rows)
    }

    /// `openWith(path, app)` -> `{ ok, error }`.
    pub fn open_with(args: &[Value]) -> Value {
        result_value(crate::fileops::open_with(&str_arg(args, 0), &str_arg(args, 1)).map(|()| String::new()))
    }

    /// `quickLook(path)` -> whether the preview is open: shows `path` (or switches to it), or
    /// closes the preview when `path` is empty.
    pub fn quick_look(args: &[Value]) -> Value {
        Value::Bool(mac::quick_look(&str_arg(args, 0)))
    }

    pub fn quick_look_visible(_a: &[Value]) -> Value {
        Value::Bool(mac::quick_look_visible())
    }

    static SUGGEST_GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    /// Typing pauses this long before a suggestion request goes out.
    const SUGGEST_DELAY: std::time::Duration = std::time::Duration::from_millis(120);

    thread_local! {
        static ON_SUGGEST: std::cell::RefCell<Option<Value>> = const { std::cell::RefCell::new(None) };
    }

    /// `webSuggest(url, query, cb)`: fetches OpenSearch suggestions from `url` on a worker thread
    /// once typing pauses; `cb({ query, results, error })` unless a newer request started.
    pub fn web_suggest(args: &[Value]) -> Value {
        use std::sync::atomic::Ordering;
        let (url, query) = (str_arg(args, 0), str_arg(args, 1));
        ON_SUGGEST.with(|c| *c.borrow_mut() = args.get(2).cloned());
        let generation = SUGGEST_GEN.fetch_add(1, Ordering::SeqCst) + 1;
        std::thread::spawn(move || {
            std::thread::sleep(SUGGEST_DELAY);
            if SUGGEST_GEN.load(Ordering::SeqCst) != generation {
                return;
            }
            let got = crate::websearch::suggest(&url, &query);
            dispatch2::DispatchQueue::main().exec_async(move || {
                if SUGGEST_GEN.load(Ordering::SeqCst) != generation {
                    return;
                }
                let Some(Value::Function(f)) = ON_SUGGEST.with(|c| c.borrow().clone()) else { return };
                let (results, error) = match got {
                    Ok(list) => (list, String::new()),
                    Err(e) => (Vec::new(), e),
                };
                let payload = obj(vec![
                    ("query", s(&query)),
                    ("results", arr(results.iter().map(|r| s(r)).collect())),
                    ("error", s(&error)),
                ]);
                mac::with_ui(|| {
                    let _ = f.call(&[payload]);
                });
            });
        });
        Value::Number(generation as f64)
    }

    static CONTENTS_GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    thread_local! {
        static ON_CONTENTS: std::cell::RefCell<Option<Value>> = const { std::cell::RefCell::new(None) };
    }

    /// `searchContents(text, limit, cb)`: Spotlight search of file contents on a worker thread;
    /// `cb({ query, results })` unless a newer search started. Rows look like searchFiles rows.
    pub fn search_contents(args: &[Value]) -> Value {
        use std::sync::atomic::Ordering;
        let (query, limit) = (str_arg(args, 0), num_arg(args, 1, 8.0).clamp(1.0, 50.0) as usize);
        ON_CONTENTS.with(|c| *c.borrow_mut() = args.get(2).cloned());
        let generation = CONTENTS_GEN.fetch_add(1, Ordering::SeqCst) + 1;
        std::thread::spawn(move || {
            let hits = files::search_contents(&query, limit);
            if CONTENTS_GEN.load(Ordering::SeqCst) != generation {
                return;
            }
            dispatch2::DispatchQueue::main().exec_async(move || {
                if CONTENTS_GEN.load(Ordering::SeqCst) != generation {
                    return;
                }
                let Some(Value::Function(f)) = ON_CONTENTS.with(|c| c.borrow().clone()) else { return };
                let rows = hits
                    .iter()
                    .map(|h| {
                        obj(vec![
                            ("name", s(&h.name)),
                            ("path", s(&h.path)),
                            ("icon", s(&mac::icon_name(&h.path))),
                            ("kind", s(if h.is_dir { "Folder" } else { "File" })),
                            ("detail", s(&h.detail)),
                        ])
                    })
                    .collect();
                let payload = obj(vec![("query", s(&query)), ("results", arr(rows))]);
                mac::with_ui(|| {
                    let _ = f.call(&[payload]);
                });
            });
        });
        Value::Number(generation as f64)
    }

    /// `contactsAccess()` -> notDetermined, restricted, denied, authorized or limited.
    pub fn contacts_access(_a: &[Value]) -> Value {
        s(crate::contacts::status())
    }

    thread_local! {
        static ON_CONTACTS_ACCESS: std::cell::RefCell<Option<Value>> = const { std::cell::RefCell::new(None) };
    }

    /// `contactsRequest(cb)`: shows the Contacts permission prompt the first time; `cb(granted)`
    /// runs on the main thread.
    pub fn contacts_request(args: &[Value]) -> Value {
        ON_CONTACTS_ACCESS.with(|c| *c.borrow_mut() = args.first().cloned());
        crate::contacts::request(|granted| {
            dispatch2::DispatchQueue::main().exec_async(move || {
                let Some(Value::Function(f)) = ON_CONTACTS_ACCESS.with(|c| c.borrow_mut().take()) else { return };
                mac::with_ui(|| {
                    let _ = f.call(&[Value::Bool(granted)]);
                });
            });
        });
        Value::Null
    }

    static CONTACTS_GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    thread_local! {
        static ON_CONTACTS: std::cell::RefCell<Option<Value>> = const { std::cell::RefCell::new(None) };
    }

    /// `searchContacts(text, limit, cb)`: name search on a worker thread (every contact when text
    /// is empty); `cb({ query, results: [{ id, name, given, family, org, title,
    /// emails: [{ label, value }], phones: [{ label, value }] }], error })` unless a newer search
    /// started.
    pub fn search_contacts(args: &[Value]) -> Value {
        use std::sync::atomic::Ordering;
        let (query, limit) = (str_arg(args, 0), num_arg(args, 1, 20.0).clamp(1.0, 500.0) as usize);
        ON_CONTACTS.with(|c| *c.borrow_mut() = args.get(2).cloned());
        let generation = CONTACTS_GEN.fetch_add(1, Ordering::SeqCst) + 1;
        std::thread::spawn(move || {
            let got = crate::contacts::search(&query, limit);
            dispatch2::DispatchQueue::main().exec_async(move || {
                if CONTACTS_GEN.load(Ordering::SeqCst) != generation {
                    return;
                }
                let Some(Value::Function(f)) = ON_CONTACTS.with(|c| c.borrow().clone()) else { return };
                let fields = |list: &[crate::contacts::Field]| {
                    arr(list.iter().map(|x| obj(vec![("label", s(&x.label)), ("value", s(&x.value))])).collect())
                };
                let (rows, error) = match got {
                    Ok(list) => (
                        list.iter()
                            .map(|c| {
                                obj(vec![
                                    ("id", s(&c.id)),
                                    ("name", s(&c.name())),
                                    ("given", s(&c.given)),
                                    ("family", s(&c.family)),
                                    ("org", s(&c.org)),
                                    ("title", s(&c.title)),
                                    ("emails", fields(&c.emails)),
                                    ("phones", fields(&c.phones)),
                                ])
                            })
                            .collect(),
                        String::new(),
                    ),
                    Err(e) => (Vec::new(), e),
                };
                let payload = obj(vec![("query", s(&query)), ("results", arr(rows)), ("error", s(&error))]);
                mac::with_ui(|| {
                    let _ = f.call(&[payload]);
                });
            });
        });
        Value::Number(generation as f64)
    }

    /// `watchSnippets(keywords, cb)` -> `{ ok, error }`: call `cb(keyword)` when one of the
    /// keywords is typed in another app. Asks for Accessibility when there are keywords to watch.
    pub fn watch_snippets(args: &[Value]) -> Value {
        let keywords: Vec<String> = match args.first() {
            Some(Value::Array(a)) => a.borrow().iter().filter_map(|v| if let Value::String(k) = v { Some(k.to_string()) } else { None }).collect(),
            _ => Vec::new(),
        };
        let watching = !keywords.is_empty();
        mac::watch_snippets(keywords, args.get(1).cloned().unwrap_or(Value::Null));
        if watching && !crate::ax::trusted(false) {
            crate::ax::trusted(true);
            return obj(vec![("ok", Value::Bool(false)), ("error", s(crate::ax::NEEDS_PERMISSION))]);
        }
        obj(vec![("ok", Value::Bool(true))])
    }

    /// `replaceTyped(keyword, text)` -> `{ ok, error }`: replace the keyword just typed before the
    /// cursor in the focused app with the text.
    pub fn replace_typed(args: &[Value]) -> Value {
        match crate::ax::replace_typed(&str_arg(args, 0), &str_arg(args, 1)) {
            Ok(()) => obj(vec![("ok", Value::Bool(true))]),
            Err(e) => obj(vec![("ok", Value::Bool(false)), ("error", s(&e))]),
        }
    }

    /// `selectedText()` -> the selected text in the focused app, or null.
    pub fn selected_text(_a: &[Value]) -> Value {
        crate::ax::selected_text().map(|t| s(&t)).unwrap_or(Value::Null)
    }

    /// `accessibilityTrusted(prompt)` -> whether Moo may use Accessibility; `prompt` shows the
    /// macOS dialog.
    pub fn accessibility_trusted(args: &[Value]) -> Value {
        Value::Bool(crate::ax::trusted(matches!(args.first(), Some(Value::Bool(true)))))
    }

    /// `volume()` -> `{ percent, muted }` or null when the output device has no volume control.
    pub fn volume(_a: &[Value]) -> Value {
        match crate::system::volume() {
            Ok((v, m)) => obj(vec![("percent", Value::Number(v)), ("muted", Value::Bool(m))]),
            Err(_) => Value::Null,
        }
    }

    /// `darkMode()` -> true, false, or null when it cannot be read.
    pub fn dark_mode(_a: &[Value]) -> Value {
        crate::system::dark_mode().map(Value::Bool).unwrap_or(Value::Null)
    }

    /// `systemInfo()` -> `{ os, model, chip, cores, memoryBytes, uptimeSecs, diskTotal, diskFree,
    /// battery: { percent, charging, onAC, minutesToEmpty, minutesToFull } | null }`.
    pub fn system_info(_a: &[Value]) -> Value {
        let i = crate::sysinfo::info();
        let n = |x: f64| Value::Number(x);
        let battery = match i.battery {
            Some(b) => obj(vec![
                ("percent", n(b.percent)),
                ("charging", Value::Bool(b.charging)),
                ("onAC", Value::Bool(b.on_ac)),
                ("minutesToEmpty", n(b.minutes_to_empty)),
                ("minutesToFull", n(b.minutes_to_full)),
            ]),
            None => Value::Null,
        };
        obj(vec![
            ("os", Value::String(i.os.as_str().into())),
            ("model", Value::String(i.model.as_str().into())),
            ("chip", Value::String(i.chip.as_str().into())),
            ("cores", n(i.cores as f64)),
            ("memoryBytes", n(i.memory_bytes as f64)),
            ("uptimeSecs", n(i.uptime_secs)),
            ("diskTotal", n(i.disk_total as f64)),
            ("diskFree", n(i.disk_free as f64)),
            ("battery", battery),
        ])
    }

    /// `fileIcon(path)` -> image name for an `<image src>`.
    pub fn file_icon(args: &[Value]) -> Value {
        Value::String(mac::icon_name(&str_arg(args, 0)).as_str().into())
    }

    /// `revealFile(path)`: select it in a Finder window.
    pub fn reveal_file(args: &[Value]) -> Value {
        let ok = mac::reveal(&str_arg(args, 0));
        if ok {
            mac::hide();
        }
        Value::Bool(ok)
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

    /// `watchApps()`: keep the app index live with FSEvents on the application folders.
    pub fn watch_apps(_a: &[Value]) -> Value {
        Value::Bool(mac::watch_apps())
    }

    /// `statusItem(hotkey, symbol, onMenu)`: menu bar icon (an SF Symbol name). A click shows the
    /// panel; a right click offers Settings… (calls `onMenu("settings")`) and Quit Moo.
    pub fn status_item(args: &[Value]) -> Value {
        thread_local! {
            static ON_MENU: std::cell::RefCell<Option<Value>> = const { std::cell::RefCell::new(None) };
        }
        ON_MENU.with(|c| *c.borrow_mut() = args.get(2).cloned());
        let handler = Box::new(|what: &str| {
            let what = what.to_string();
            dispatch2::DispatchQueue::main().exec_async(move || {
                let Some(Value::Function(f)) = ON_MENU.with(|c| c.borrow().clone()) else { return };
                mac::with_ui(|| {
                    let _ = f.call(&[s(&what)]);
                });
            });
        });
        Value::Bool(mac::status_item(&str_arg(args, 0), &str_arg(args, 1), handler))
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
    // ── Remote models (Hypery, OpenAI, Ollama, LM Studio, ai.providers) ──

    fn result(r: Result<(), String>) -> Value {
        match r {
            Ok(()) => obj(vec![("ok", Value::Bool(true)), ("error", s(""))]),
            Err(e) => obj(vec![("ok", Value::Bool(false)), ("error", s(&e))]),
        }
    }

    /// `aiProviders()` -> `[{ id, title, url, local, credential, oauth, signIn, account }]`, Apple's
    /// on-device model first. `credential`: "key", "signed in", "environment" or "".
    pub fn ai_providers(_a: &[Value]) -> Value {
        remote::providers_value()
    }

    /// `aiModels(cb)`: `cb([{ provider, title, models: [{ id, name, owner, context, tier }], error }])`.
    pub fn ai_models(args: &[Value]) -> Value {
        remote::models(callback(args, 0));
        Value::Null
    }

    /// `aiChat(model, messagesJson, toolsJson, cb)` -> request id. `model` is `provider:model`;
    /// `cb({ request, kind, text, calls: [{ id, name, arguments }], finish })`, kind "partial", then
    /// "done", "error" or "cancelled".
    pub fn ai_chat(args: &[Value]) -> Value {
        Value::Number(remote::chat(&str_arg(args, 0), &str_arg(args, 1), &str_arg(args, 2), callback(args, 3)) as f64)
    }

    pub fn ai_chat_cancel(args: &[Value]) -> Value {
        remote::cancel(num_arg(args, 0, 0.0) as u64);
        Value::Null
    }

    /// `aiSetKey(provider, key)` -> `{ ok, error }`; an empty key removes it.
    pub fn ai_set_key(args: &[Value]) -> Value {
        result(remote::set_key(&str_arg(args, 0), &str_arg(args, 1)))
    }

    /// `aiLogin(provider, cb)`: browser sign-in; `cb({ ok, error })`.
    pub fn ai_login(args: &[Value]) -> Value {
        remote::login(&str_arg(args, 0), callback(args, 1));
        Value::Null
    }

    pub fn ai_cancel_login(_a: &[Value]) -> Value {
        remote::cancel_login();
        Value::Null
    }

    /// `aiLogout(provider)`: forget its API key and sign-in.
    pub fn ai_logout(args: &[Value]) -> Value {
        remote::logout(&str_arg(args, 0));
        Value::Null
    }

    /// `aiAccount(provider, cb)`: `cb({ provider, credential, email, name, billing, keys, balance,
    /// monthSpent, monthLimit, error })`, amounts in US dollars (-1 unknown).
    pub fn ai_account(args: &[Value]) -> Value {
        remote::account(&str_arg(args, 0), callback(args, 1));
        Value::Null
    }

    /// `aiSetModel(model)` -> `{ ok, error }`: `ai.model` in shortcuts.json (`apple` or
    /// `provider:model`; empty lets Moo choose).
    pub fn ai_set_model(args: &[Value]) -> Value {
        let model = str_arg(args, 0).trim().to_string();
        edit_config(move |cfg| {
            cfg.ai.model = model;
            Ok(())
        })
    }

    /// `aiSetClientId(provider, id)` -> `{ ok, error }`: the OAuth client id for browser sign-in.
    pub fn ai_set_client_id(args: &[Value]) -> Value {
        let (provider, id) = (str_arg(args, 0), str_arg(args, 1).trim().to_string());
        edit_config(move |cfg| {
            let mut p = cfg.ai.providers.iter().find(|p| p.id == provider).cloned().unwrap_or(shortcuts::ProviderConfig { id: provider.clone(), ..Default::default() });
            p.client_id = id;
            shortcuts::set_provider(cfg, p);
            Ok(())
        })
    }

    /// `aiSetOrganization(provider, id)` -> `{ ok, error }`: the organization requests act for
    /// and bill to; "" for the personal one.
    pub fn ai_set_organization(args: &[Value]) -> Value {
        let (provider, id) = (str_arg(args, 0), str_arg(args, 1).trim().to_string());
        edit_config(move |cfg| {
            let mut p = cfg.ai.providers.iter().find(|p| p.id == provider).cloned().unwrap_or(shortcuts::ProviderConfig { id: provider.clone(), ..Default::default() });
            p.organization = id;
            shortcuts::set_provider(cfg, p);
            Ok(())
        })
    }

    /// `symbolIcon(name)` -> an image name for `<image src>` showing SF Symbol `name`.
    pub fn symbol_icon(args: &[Value]) -> Value {
        Value::String(mac::symbol_icon(&str_arg(args, 0)).as_str().into())
    }

    /// `siteIcon(url, cb)` -> an image name for `<image src>` showing the site's favicon, or ""
    /// until it is cached; `cb(name)` once a fresh copy has downloaded.
    pub fn site_icon(args: &[Value]) -> Value {
        let cb = bridge::hold(callback(args, 1));
        Value::String(siteicon::site_icon(&str_arg(args, 0), cb).as_str().into())
    }

    /// `imageIcon(url, cb)` -> an image name for `<image src>` showing the image at `url`, or ""
    /// until it is cached; `cb(name)` once a fresh copy has downloaded.
    pub fn image_icon(args: &[Value]) -> Value {
        let cb = bridge::hold(callback(args, 1));
        Value::String(siteicon::image_icon(&str_arg(args, 0), cb).as_str().into())
    }

    /// `imageFile(path, template)` -> an image name for `<image src>` (or `statusItem`) showing
    /// the image file at `path`, or "" when there is no image there. A `template` image takes the
    /// view's `tint`, like an SF Symbol.
    pub fn image_file(args: &[Value]) -> Value {
        let template = matches!(args.get(1), Some(Value::Bool(true)));
        Value::String(siteicon::image_file(&str_arg(args, 0), template).as_str().into())
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
    pub fn launch(_a: &[Value]) -> Value { Value::Bool(false) }
    pub fn copy_text(_a: &[Value]) -> Value { Value::Bool(false) }
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
    fn unsupported(_a: &[Value]) -> Value {
        obj(vec![("ok", Value::Bool(false)), ("error", Value::String("macOS only".into()))])
    }
    pub use unsupported as unregister_hotkey;
    pub use unsupported as check_hotkey;
    pub use unsupported as load_shortcuts;
    pub use unsupported as save_shortcut;
    pub use unsupported as check_keyword;
    pub use unsupported as remove_shortcut;
    pub use unsupported as bind_hotkey;
    pub use unsupported as unbind_hotkey;
    pub use unsupported as set_launcher_hotkey;
    pub use unsupported as cli_serve;
    pub fn hotkey_display(a: &[Value]) -> Value { Value::String(str_arg(a, 0).as_str().into()) }
    pub fn ensure_shortcuts_file(_a: &[Value]) -> Value { Value::String("".into()) }
    pub fn toggle(_a: &[Value]) -> Value { Value::Null }
    pub fn record_hotkey(_a: &[Value]) -> Value { Value::Null }
    pub fn expand_template(a: &[Value]) -> Value { Value::String(str_arg(a, 0).as_str().into()) }
    pub fn watch_shortcuts(_a: &[Value]) -> Value { Value::Bool(false) }
    pub fn run_shell(_a: &[Value]) -> Value { Value::Number(0.0) }
    pub fn clipboard_text(_a: &[Value]) -> Value { Value::String("".into()) }
    pub fn cli_main(_a: &[Value]) -> Value { Value::Bool(false) }
    pub fn cli_write(_a: &[Value]) -> Value { Value::Bool(false) }
    pub fn cli_end(_a: &[Value]) -> Value { Value::Bool(false) }
    pub fn cli_usage(_a: &[Value]) -> Value { Value::String("".into()) }
    pub fn quit(_a: &[Value]) -> Value { Value::Null }
    pub fn search_files(_a: &[Value]) -> Value { Value::Number(0.0) }
    pub fn file_index_start(_a: &[Value]) -> Value { Value::Bool(false) }
    pub fn find_files(_a: &[Value]) -> Value {
        obj(vec![("ready", Value::Bool(false)), ("results", Value::Array(VmRef::new(vec![]))), ("ms", Value::Number(0.0))])
    }
    pub fn recent_files(_a: &[Value]) -> Value { Value::Array(VmRef::new(vec![])) }
    pub fn query_files(_a: &[Value]) -> Value {
        obj(vec![("results", Value::Array(VmRef::new(vec![]))), ("total", Value::Number(0.0)), ("ms", Value::Number(0.0))])
    }
    pub fn running_apps(_a: &[Value]) -> Value { Value::Array(VmRef::new(vec![])) }
    pub fn define(_a: &[Value]) -> Value { Value::Null }
    pub fn dictionary_warm(_a: &[Value]) -> Value { Value::Null }
    pub use unsupported as app_action;
    pub fn calculate(a: &[Value]) -> Value {
        match calc::answer(&str_arg(a, 0), None) {
            Some(x) => obj(vec![("display", Value::String(x.display.as_str().into())), ("copy", Value::String(x.copy.as_str().into())), ("detail", Value::String(x.detail.as_str().into()))]),
            None => Value::Null,
        }
    }
    pub fn system_info(_a: &[Value]) -> Value { Value::Null }
    pub fn system_command(_a: &[Value]) -> Value { Value::Null }
    pub fn volume(_a: &[Value]) -> Value { Value::Null }
    pub use unsupported as arrange_window;
    pub use unsupported as watch_snippets;
    pub use unsupported as replace_typed;
    pub use unsupported as trash_file;
    pub use unsupported as open_with;
    pub fn apps_for(_a: &[Value]) -> Value { Value::Array(VmRef::new(vec![])) }
    pub fn quick_look(_a: &[Value]) -> Value { Value::Bool(false) }
    pub fn quick_look_visible(_a: &[Value]) -> Value { Value::Bool(false) }
    pub fn search_contents(_a: &[Value]) -> Value { Value::Number(0.0) }
    pub fn web_suggest(_a: &[Value]) -> Value { Value::Number(0.0) }
    pub fn contacts_access(_a: &[Value]) -> Value { Value::String("restricted".into()) }
    pub fn contacts_request(_a: &[Value]) -> Value { Value::Null }
    pub fn search_contacts(_a: &[Value]) -> Value { Value::Number(0.0) }
    pub fn selected_text(_a: &[Value]) -> Value { Value::Null }
    pub fn accessibility_trusted(_a: &[Value]) -> Value { Value::Bool(false) }
    pub fn dark_mode(_a: &[Value]) -> Value { Value::Null }
    pub fn file_icon(_a: &[Value]) -> Value { Value::String("".into()) }
    pub fn reveal_file(_a: &[Value]) -> Value { Value::Bool(false) }
    pub fn file_index_status(_a: &[Value]) -> Value { obj(vec![("state", Value::String("idle".into()))]) }
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
    pub fn ai_providers(_a: &[Value]) -> Value { Value::Array(VmRef::new(Vec::new())) }
    pub fn ai_chat(_a: &[Value]) -> Value { Value::Number(0.0) }
    pub use ai_cancel as ai_models;
    pub use ai_cancel as ai_chat_cancel;
    pub use ai_cancel as ai_login;
    pub use ai_cancel as ai_cancel_login;
    pub use ai_cancel as ai_logout;
    pub use ai_cancel as ai_account;
    pub use unsupported as ai_set_key;
    pub use unsupported as ai_set_model;
    pub use unsupported as ai_set_client_id;
    pub use unsupported as ai_set_organization;
    pub fn symbol_icon(_a: &[Value]) -> Value { Value::String("".into()) }
    pub fn site_icon(_a: &[Value]) -> Value { Value::String("".into()) }
    pub use site_icon as image_icon;
    pub use symbol_icon as image_file;
    pub fn on_plugin_refresh(_a: &[Value]) -> Value { Value::Null }
}

pub fn moo_object() -> Value {
    prefs::migrate_legacy_dirs();
    let mut m = ObjectMap::default();
    m.insert(Arc::from("reindex"), Value::native(native_reindex));
    m.insert(Arc::from("search"), Value::native(native_search));
    m.insert(Arc::from("launch"), Value::native(natives::launch));
    m.insert(Arc::from("setup"), Value::native(natives::setup));
    m.insert(Arc::from("setPanelShape"), Value::native(natives::set_panel_shape));
    m.insert(Arc::from("searchHistory"), Value::native(native_search_history));
    m.insert(Arc::from("addSearchHistory"), Value::native(native_add_search_history));
    m.insert(Arc::from("clearSearchHistory"), Value::native(native_clear_search_history));
    m.insert(Arc::from("getPref"), Value::native(native_get_pref));
    m.insert(Arc::from("setPref"), Value::native(native_set_pref));
    m.insert(Arc::from("setArrowKeys"), Value::native(natives::set_arrow_keys));
    m.insert(Arc::from("typeText"), Value::native(natives::type_text));
    m.insert(Arc::from("setTheme"), Value::native(natives::set_theme));
    m.insert(Arc::from("registerHotkey"), Value::native(natives::register_hotkey));
    m.insert(Arc::from("unregisterHotkey"), Value::native(natives::unregister_hotkey));
    m.insert(Arc::from("checkHotkey"), Value::native(natives::check_hotkey));
    m.insert(Arc::from("hotkeyDisplay"), Value::native(natives::hotkey_display));
    m.insert(Arc::from("recordHotkey"), Value::native(natives::record_hotkey));
    m.insert(Arc::from("loadShortcuts"), Value::native(natives::load_shortcuts));
    m.insert(Arc::from("saveShortcut"), Value::native(natives::save_shortcut));
    m.insert(Arc::from("checkKeyword"), Value::native(natives::check_keyword));
    m.insert(Arc::from("ensureShortcutsFile"), Value::native(natives::ensure_shortcuts_file));
    m.insert(Arc::from("toggle"), Value::native(natives::toggle));
    m.insert(Arc::from("removeShortcut"), Value::native(natives::remove_shortcut));
    m.insert(Arc::from("bindHotkey"), Value::native(natives::bind_hotkey));
    m.insert(Arc::from("unbindHotkey"), Value::native(natives::unbind_hotkey));
    m.insert(Arc::from("setLauncherHotkey"), Value::native(natives::set_launcher_hotkey));
    m.insert(Arc::from("expandTemplate"), Value::native(natives::expand_template));
    m.insert(Arc::from("watchShortcuts"), Value::native(natives::watch_shortcuts));
    m.insert(Arc::from("runShell"), Value::native(natives::run_shell));
    m.insert(Arc::from("clipboardText"), Value::native(natives::clipboard_text));
    m.insert(Arc::from("cliMain"), Value::native(natives::cli_main));
    m.insert(Arc::from("cliServe"), Value::native(natives::cli_serve));
    m.insert(Arc::from("cliWrite"), Value::native(natives::cli_write));
    m.insert(Arc::from("cliEnd"), Value::native(natives::cli_end));
    m.insert(Arc::from("cliUsage"), Value::native(natives::cli_usage));
    m.insert(Arc::from("show"), Value::native(natives::show));
    m.insert(Arc::from("hide"), Value::native(natives::hide));
    m.insert(Arc::from("quit"), Value::native(natives::quit));
    m.insert(Arc::from("appCount"), Value::native(native_app_count));
    m.insert(Arc::from("fuzzy"), Value::native(native_fuzzy));
    m.insert(Arc::from("pluginPaths"), Value::native(native_plugin_paths));
    m.insert(Arc::from("copyText"), Value::native(natives::copy_text));
    m.insert(Arc::from("loadBytecodePlugin"), Value::native(native_load_bytecode_plugin));
    m.insert(Arc::from("searchFiles"), Value::native(natives::search_files));
    m.insert(Arc::from("fileIndexStart"), Value::native(natives::file_index_start));
    m.insert(Arc::from("findFiles"), Value::native(natives::find_files));
    m.insert(Arc::from("recentFiles"), Value::native(natives::recent_files));
    m.insert(Arc::from("queryFiles"), Value::native(natives::query_files));
    m.insert(Arc::from("runningApps"), Value::native(natives::running_apps));
    m.insert(Arc::from("appAction"), Value::native(natives::app_action));
    m.insert(Arc::from("calculate"), Value::native(natives::calculate));
    m.insert(Arc::from("define"), Value::native(natives::define));
    m.insert(Arc::from("webSuggest"), Value::native(natives::web_suggest));
    m.insert(Arc::from("contactsAccess"), Value::native(natives::contacts_access));
    m.insert(Arc::from("contactsRequest"), Value::native(natives::contacts_request));
    m.insert(Arc::from("searchContacts"), Value::native(natives::search_contacts));
    m.insert(Arc::from("dictionaryWarm"), Value::native(natives::dictionary_warm));
    m.insert(Arc::from("systemInfo"), Value::native(natives::system_info));
    m.insert(Arc::from("systemCommand"), Value::native(natives::system_command));
    m.insert(Arc::from("volume"), Value::native(natives::volume));
    m.insert(Arc::from("arrangeWindow"), Value::native(natives::arrange_window));
    m.insert(Arc::from("selectedText"), Value::native(natives::selected_text));
    m.insert(Arc::from("watchSnippets"), Value::native(natives::watch_snippets));
    m.insert(Arc::from("trashFile"), Value::native(natives::trash_file));
    m.insert(Arc::from("appsFor"), Value::native(natives::apps_for));
    m.insert(Arc::from("openWith"), Value::native(natives::open_with));
    m.insert(Arc::from("quickLook"), Value::native(natives::quick_look));
    m.insert(Arc::from("quickLookVisible"), Value::native(natives::quick_look_visible));
    m.insert(Arc::from("searchContents"), Value::native(natives::search_contents));
    m.insert(Arc::from("replaceTyped"), Value::native(natives::replace_typed));
    m.insert(Arc::from("accessibilityTrusted"), Value::native(natives::accessibility_trusted));
    m.insert(Arc::from("darkMode"), Value::native(natives::dark_mode));
    m.insert(Arc::from("fileIcon"), Value::native(natives::file_icon));
    m.insert(Arc::from("revealFile"), Value::native(natives::reveal_file));
    m.insert(Arc::from("fileIndexStatus"), Value::native(natives::file_index_status));
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
    m.insert(Arc::from("aiProviders"), Value::native(natives::ai_providers));
    m.insert(Arc::from("aiModels"), Value::native(natives::ai_models));
    m.insert(Arc::from("aiChat"), Value::native(natives::ai_chat));
    m.insert(Arc::from("aiChatCancel"), Value::native(natives::ai_chat_cancel));
    m.insert(Arc::from("aiSetKey"), Value::native(natives::ai_set_key));
    m.insert(Arc::from("aiLogin"), Value::native(natives::ai_login));
    m.insert(Arc::from("aiCancelLogin"), Value::native(natives::ai_cancel_login));
    m.insert(Arc::from("aiLogout"), Value::native(natives::ai_logout));
    m.insert(Arc::from("aiAccount"), Value::native(natives::ai_account));
    m.insert(Arc::from("aiSetModel"), Value::native(natives::ai_set_model));
    m.insert(Arc::from("aiSetClientId"), Value::native(natives::ai_set_client_id));
    m.insert(Arc::from("symbolIcon"), Value::native(natives::symbol_icon));
    m.insert(Arc::from("siteIcon"), Value::native(natives::site_icon));
    m.insert(Arc::from("imageIcon"), Value::native(natives::image_icon));
    m.insert(Arc::from("imageFile"), Value::native(natives::image_file));
    m.insert(Arc::from("onPluginRefresh"), Value::native(natives::on_plugin_refresh));
    m.insert(Arc::from("aiSetOrganization"), Value::native(natives::ai_set_organization));
    Value::object(m)
}
