//! Tier A plugins: a serialized bytecode chunk run in its own `tish_vm` with an empty capability
//! set (no fs, process, network or timers). The chunk hands its exports to the host by calling the
//! injected `register({ manifest, run, list })`; the returned closures are ordinary `Value`s, so
//! the shell calls them exactly like a native plugin's exports.

use std::cell::RefCell;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

use tishlang_bytecode::Chunk;
use tishlang_core::Value;
use tishlang_vm::Vm;

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
    let exports = run_chunk(&chunk).map_err(|e| format!("{path}: {e}"))?;
    Ok((exports, t0.elapsed().as_secs_f64() * 1000.0))
}

fn run_chunk(chunk: &Chunk) -> Result<Value, String> {
    let mut vm = Vm::with_capabilities(HashSet::new());
    vm.set_global(Arc::from("register"), Value::native(register));
    REGISTERED.with(|r| r.borrow_mut().take());
    vm.run(chunk)?;
    REGISTERED
        .with(|r| r.borrow_mut().take())
        .ok_or_else(|| "plugin never called register()".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load_src(src: &str) -> Result<Value, String> {
        let program = tishlang_parser::parse(src).map_err(|e| format!("{e:?}"))?;
        let chunk = tishlang_bytecode::compile(&program).map_err(|e| format!("{e:?}"))?;
        run_chunk(&chunk)
    }

    fn call(exports: &Value, name: &str, args: &[Value]) -> Value {
        let Value::Object(o) = exports else { panic!("exports not an object") };
        let f = o.borrow().strings.get(name).cloned();
        match f {
            Some(Value::Function(f)) => f.call(args),
            other => panic!("{name} is not a function: {other:?}"),
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
}
