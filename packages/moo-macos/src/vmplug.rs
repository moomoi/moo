//! Tier A plugins: a serialized bytecode chunk run in its own `tish_vm` with an empty capability
//! set (no fs, process, network or timers). The chunk hands its exports to the host by calling the
//! injected `register({ manifest, run, list })`; the returned closures are ordinary `Value`s, so
//! the shell calls them exactly like a native plugin's exports.
//!
//! Plugins share the shell's process and main thread, so each VM runs with the JIT off (the JIT is
//! process-global and its compiled loops never poll a deadline) and every call is budgeted by a
//! per-thread execution deadline. A runaway plugin raises a catchable error instead of hanging the
//! launcher.
//!
//! The VM's only way out is the `moo` global (`pluginhost`): fetch, storage, sign-in, limited to
//! the hosts the manifest declares in `permissions.network`.

use std::cell::RefCell;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

use tishlang_bytecode::Chunk;
use tishlang_core::{
    execution_deadline_tripped, set_pending_throw, set_thread_execution_deadline, take_pending_throw, ObjectMap,
    Value,
};
use tishlang_vm::Vm;

/// Budget for running the chunk's top level (setup plus `register`).
const LOAD_BUDGET_MS: u64 = 1000;
/// Budget for one call into an export. `list` runs on every keystroke, so keep this well under a frame
/// budget the user would notice as a stall.
const CALL_BUDGET_MS: u64 = 250;

thread_local! {
    static REGISTERED: RefCell<Option<Value>> = const { RefCell::new(None) };
}

fn register(args: &[Value]) -> Value {
    REGISTERED.with(|r| *r.borrow_mut() = args.first().cloned());
    Value::Null
}

/// Load and run a chunk file; returns the registered exports and the load time in milliseconds.
pub fn load(path: &str) -> Result<(Value, f64), String> {
    let t0 = Instant::now();
    let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    let chunk = tishlang_bytecode::deserialize(&bytes).map_err(|e| format!("{path}: {e}"))?;
    let name = std::path::Path::new(path).file_stem().map_or(path.into(), |s| s.to_string_lossy());
    let exports = run_chunk(&chunk, &name).map_err(|e| format!("{path}: {e}"))?;
    Ok((exports, t0.elapsed().as_secs_f64() * 1000.0))
}

fn run_chunk(chunk: &Chunk, label: &str) -> Result<Value, String> {
    let mut vm = Vm::with_capabilities(HashSet::new());
    vm.set_jit_enabled(false);
    vm.set_global(Arc::from("register"), Value::native(register));
    #[cfg(target_os = "macos")]
    let host: crate::pluginhost::Shared = Arc::new(std::sync::Mutex::new(crate::pluginhost::Host { id: label.to_string(), network: Vec::new(), sign_in_cancel: None }));
    #[cfg(target_os = "macos")]
    vm.set_global(Arc::from("moo"), crate::pluginhost::object(host.clone()));
    REGISTERED.with(|r| r.borrow_mut().take());
    set_thread_execution_deadline(Some(LOAD_BUDGET_MS));
    let result = vm.run(chunk);
    let tripped = execution_deadline_tripped();
    set_thread_execution_deadline(None);
    if tripped {
        take_pending_throw();
        return Err(format!("top level exceeded its {LOAD_BUDGET_MS} ms budget"));
    }
    result?;
    let exports = REGISTERED
        .with(|r| r.borrow_mut().take())
        .ok_or_else(|| "plugin never called register()".to_string())?;
    let exports = budgeted_exports(&exports, label);
    #[cfg(target_os = "macos")]
    if let Some(Value::Function(manifest)) = field(&exports, "manifest") {
        let m = manifest.call(&[]);
        take_pending_throw();
        let mut h = host.lock().unwrap_or_else(|e| e.into_inner());
        h.network = crate::pluginhost::network_permissions(&m);
        // Identity is the installed file's name (`<id>.tishc`), never what the plugin says about
        // itself: an id taken from `manifest()` would let a plugin claim another's Keychain secrets
        // and store. A manifest that disagrees doesn't load.
        if let Some(Value::String(id)) = field(&m, "id") {
            if &*id != label {
                return Err(format!("manifest id \"{id}\" does not match the file name \"{label}\""));
            }
        }
    }
    Ok(exports)
}

#[cfg(target_os = "macos")]
fn field(v: &Value, key: &str) -> Option<Value> {
    match v {
        Value::Object(o) => o.borrow().strings.get(key).cloned(),
        _ => None,
    }
}

/// Copy of `exports` whose functions run under [`CALL_BUDGET_MS`]; other fields pass through.
fn budgeted_exports(exports: &Value, label: &str) -> Value {
    let Value::Object(o) = exports else { return exports.clone() };
    let mut out = ObjectMap::default();
    for (key, value) in o.borrow().strings.iter() {
        let wrapped = match value {
            Value::Function(_) => budgeted(value.clone(), format!("{label}: {key}()")),
            other => other.clone(),
        };
        out.insert(key.clone(), wrapped);
    }
    Value::object(out)
}

pub(crate) fn budgeted(f: Value, what: String) -> Value {
    Value::native(move |args: &[Value]| {
        let Value::Function(inner) = &f else { return Value::Null };
        set_thread_execution_deadline(Some(CALL_BUDGET_MS));
        let result = inner.call(args);
        let tripped = execution_deadline_tripped();
        set_thread_execution_deadline(None);
        if tripped {
            take_pending_throw();
            set_pending_throw(error(&format!("{what} exceeded its {CALL_BUDGET_MS} ms budget")));
            return Value::Null;
        }
        result
    })
}

fn error(message: &str) -> Value {
    let mut m = ObjectMap::default();
    m.insert(Arc::from("name"), Value::String("Error".into()));
    m.insert(Arc::from("message"), Value::String(message.into()));
    Value::object(m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn load_src(src: &str) -> Result<Value, String> {
        let program = tishlang_parser::parse(src).map_err(|e| format!("{e:?}"))?;
        let chunk = tishlang_bytecode::compile(&program).map_err(|e| format!("{e:?}"))?;
        run_chunk(&chunk, "test")
    }

    /// A plugin can't take another's identity: its manifest id must be its file name.
    #[cfg(target_os = "macos")]
    #[test]
    fn manifest_id_must_match_the_file_name() {
        let src = |id: &str| format!("register({{ manifest: () => ({{ id: \"{id}\", title: \"T\", commands: [] }}), run: (c) => null, list: (c, q) => [] }})");
        let program = tishlang_parser::parse(&src("test")).unwrap();
        let chunk = tishlang_bytecode::compile(&program).unwrap();
        assert!(run_chunk(&chunk, "test").is_ok(), "matching id loads");
        let program = tishlang_parser::parse(&src("slack")).unwrap();
        let chunk = tishlang_bytecode::compile(&program).unwrap();
        let err = run_chunk(&chunk, "test").err().expect("a plugin claiming slack must not load");
        assert!(err.contains("does not match"), "{err}");
    }

    fn call(exports: &Value, name: &str, args: &[Value]) -> Value {
        let Value::Object(o) = exports else { panic!("exports not an object") };
        let f = o.borrow().strings.get(name).cloned();
        match f {
            Some(Value::Function(f)) => f.call(args),
            other => panic!("{name} is not a function: {other:?}"),
        }
    }

    fn message(v: &Value) -> String {
        match v {
            Value::Object(o) => o.borrow().strings.get("message").map(|m| m.to_display_string()).unwrap_or_default(),
            other => other.to_display_string(),
        }
    }

    #[test]
    fn registered_closures_are_callable_and_keep_state() {
        let exports = load_src(
            "let n = 0\nregister({ bump: (by) => { n = n + by; return n } })",
        )
        .expect("load");
        assert!(matches!(call(&exports, "bump", &[Value::Number(2.0)]), Value::Number(n) if n == 2.0));
        assert!(matches!(call(&exports, "bump", &[Value::Number(3.0)]), Value::Number(n) if n == 5.0));
    }

    #[test]
    fn missing_register_is_an_error() {
        let err = load_src("let x = 1").unwrap_err();
        assert!(err.contains("register"), "{err}");
    }

    #[test]
    fn sandbox_denies_builtin_capabilities() {
        for spec in ["tish:fs", "tish:process", "tish:http", "tish:ffi"] {
            let src = format!("import {{ x }} from \"{spec}\"\nregister({{}})");
            let result = load_src(&src);
            assert!(result.is_err(), "{spec} must be denied in a Tier A VM, got {result:?}");
        }
    }

    #[test]
    fn runaway_call_throws_and_the_plugin_stays_usable() {
        let exports = load_src(
            "register({ spin: () => { let i = 0\n while (true) { i = i + 1 }\n return i }, ok: () => 7 })",
        )
        .expect("load");
        let t0 = Instant::now();
        let r = call(&exports, "spin", &[]);
        let elapsed = t0.elapsed();
        assert!(matches!(r, Value::Null), "{r:?}");
        let thrown = take_pending_throw().expect("a runaway call must leave a pending throw");
        assert!(message(&thrown).contains("spin() exceeded its 250 ms budget"), "{}", message(&thrown));
        assert!(elapsed < Duration::from_millis(1000), "took {elapsed:?}");
        assert!(matches!(call(&exports, "ok", &[]), Value::Number(n) if n == 7.0));
        assert!(take_pending_throw().is_none());
    }

    #[test]
    fn loop_free_recursion_is_budgeted_too() {
        let exports = load_src("fn fib(n) { return n < 2 ? n : fib(n - 1) + fib(n - 2) }\nregister({ fib: fib })")
            .expect("load");
        let t0 = Instant::now();
        call(&exports, "fib", &[Value::Number(45.0)]);
        let elapsed = t0.elapsed();
        let thrown = take_pending_throw().expect("fib(45) must hit the budget");
        assert!(message(&thrown).contains("budget"), "{}", message(&thrown));
        assert!(elapsed < Duration::from_millis(1000), "took {elapsed:?}");
    }

    fn get(v: &Value, key: &str) -> Value {
        match v {
            Value::Object(o) => o.borrow().strings.get(key).cloned().unwrap_or(Value::Null),
            _ => Value::Null,
        }
    }

    fn items(v: &Value) -> Vec<Value> {
        match v {
            Value::Array(a) => a.borrow().clone(),
            _ => vec![],
        }
    }

    /// Every element with `tag`, depth first.
    fn find_all(tree: &Value, tag: &str, out: &mut Vec<Value>) {
        if get(tree, "tag").to_display_string() == tag {
            out.push(tree.clone());
        }
        for c in items(&get(tree, "children")) {
            find_all(&c, tag, out);
        }
    }

    fn titles(tree: &Value) -> Vec<String> {
        let mut found = vec![];
        find_all(tree, "listitem", &mut found);
        found.iter().map(|i| get(&get(i, "props"), "title").to_display_string()).collect()
    }

    fn id(v: &Value) -> Value {
        get(v, "id")
    }

    /// Lattish views from @moo/ui, end to end in a Tier A VM. Needs `scripts/fetch-plugins.sh` first.
    #[test]
    fn lattish_view_plugin_opens_and_dispatches() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../dist/plugins/hello-list.tishc");
        if !std::path::Path::new(path).exists() {
            eprintln!("skip: {path} not built");
            return;
        }
        let (exports, ms) = load(path).expect("load");
        eprintln!("hello-list loaded in {ms:.2} ms");

        let opened = call(&exports, "open", &[Value::String("hello".into())]);
        assert!(take_pending_throw().is_none());
        let tree = get(&opened, "tree");
        let t = titles(&tree);
        assert_eq!(t.len(), 12, "{t:?}");
        assert_eq!(t[0], "Hello");

        let mut found = vec![];
        find_all(&tree, "listitem", &mut found);
        let hola = found.iter().find(|i| get(&get(i, "props"), "title").to_display_string() == "Hola").unwrap();
        let events = items(&get(hola, "events"));
        assert!(events.iter().any(|e| e.to_display_string() == "onAction"), "{events:?}");

        let view = get(&opened, "view");
        let t0 = Instant::now();
        let r = call(&exports, "dispatch", &[view.clone(), id(hola), Value::String("onAction".into()), Value::Null]);
        eprintln!("dispatch + re-render in {:.2} ms", t0.elapsed().as_secs_f64() * 1000.0);
        assert!(take_pending_throw().is_none());
        assert_eq!(get(&get(&r, "result"), "hud").to_display_string(), "Starred Hola");
        let t = titles(&get(&r, "tree"));
        assert_eq!(t[0], "★ Hola", "{t:?}");
        let mut lists = vec![];
        find_all(&get(&r, "tree"), "list", &mut lists);
        assert_eq!(get(&get(&lists[0], "props"), "navigationTitle").to_display_string(), "Hello List · 1 starred");

        let opened = call(&exports, "open", &[Value::String("change-case".into())]);
        let tree = get(&opened, "tree");
        assert!(titles(&tree).is_empty());
        let mut lists = vec![];
        find_all(&tree, "list", &mut lists);
        let r = call(
            &exports,
            "dispatch",
            &[get(&opened, "view"), id(&lists[0]), Value::String("onSearchTextChange".into()), Value::String("hello big World".into())],
        );
        assert!(take_pending_throw().is_none());
        let t = titles(&get(&r, "tree"));
        assert!(t.contains(&"helloBigWorld".to_string()), "{t:?}");
        assert!(t.contains(&"hello_big_world".to_string()), "{t:?}");

        call(&exports, "open", &[Value::String("missing".into())]);
        let thrown = take_pending_throw().expect("unknown view must throw");
        assert!(message(&thrown).contains("no view"), "{}", message(&thrown));
    }

    /// The Slack plugin's commands, their arguments and its network permission, as the shell reads
    /// them. Only the manifest: no Keychain, no network. Needs `scripts/fetch-plugins.sh` first.
    #[cfg(target_os = "macos")]
    #[test]
    fn slack_plugin_declares_arguments_and_network() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../dist/plugins/slack.tishc");
        if !std::path::Path::new(path).exists() {
            eprintln!("skip: {path} not built");
            return;
        }
        let (exports, _) = load(path).expect("load");
        for f in ["suggest", "blocker", "visible"] {
            assert!(matches!(get(&exports, f), Value::Function(_)), "{f}");
        }
        let m = call(&exports, "manifest", &[]);
        assert!(take_pending_throw().is_none());
        assert_eq!(crate::pluginhost::network_permissions(&m), ["slack.com"]);
        let commands = items(&get(&m, "commands"));
        let send = commands.iter().find(|c| get(c, "name").to_display_string() == "send").expect("send command");
        assert_eq!(get(send, "keyword").to_display_string(), "slack");
        let args: Vec<(String, String)> = items(&get(send, "arguments"))
            .iter()
            .map(|a| (get(a, "name").to_display_string(), get(a, "type").to_display_string()))
            .collect();
        assert_eq!(args, [("to".to_string(), "dropdown".to_string()), ("message".to_string(), "text".to_string())]);
        let suggested = call(&exports, "suggest", &[Value::String("search".into()), Value::String("query".into()), Value::String("x".into()), Value::Null]);
        assert!(take_pending_throw().is_none());
        assert!(items(&suggested).is_empty(), "only Send Message's To has suggestions");
    }

    #[test]
    fn runaway_top_level_is_a_load_error() {
        let t0 = Instant::now();
        let err = load_src("while (true) {}\nregister({})").unwrap_err();
        assert!(err.contains("budget"), "{err}");
        assert!(t0.elapsed() < Duration::from_millis(3000), "took {:?}", t0.elapsed());
        assert!(take_pending_throw().is_none());
    }
}
