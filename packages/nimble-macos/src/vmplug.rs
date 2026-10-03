//! Tier A plugins: a serialized bytecode chunk run in its own `tish_vm` with an empty capability
//! set (no fs, process, network or timers). The chunk hands its exports to the host by calling the
//! injected `register({ manifest, run, list })`; the returned closures are ordinary `Value`s, so
//! the shell calls them exactly like a native plugin's exports.
//!
//! Plugins share the shell's process and main thread, so each VM runs with the JIT off (the JIT is
//! process-global and its compiled loops never poll a deadline) and every call is budgeted by a
//! per-thread execution deadline. A runaway plugin raises a catchable error instead of hanging the
//! launcher.

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
    Ok(budgeted_exports(&exports, label))
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

fn budgeted(f: Value, what: String) -> Value {
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

    #[test]
    fn runaway_top_level_is_a_load_error() {
        let t0 = Instant::now();
        let err = load_src("while (true) {}\nregister({})").unwrap_err();
        assert!(err.contains("budget"), "{err}");
        assert!(t0.elapsed() < Duration::from_millis(3000), "took {:?}", t0.elapsed());
        assert!(take_pending_throw().is_none());
    }
}
