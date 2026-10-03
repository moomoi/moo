//! Decoupled C-ABI wrapper (tish_ffi v1) around a Tish module emitted with `--target rust-lib`.
//!
//! The plugin links its own `tishlang_runtime`; host values are only touched through the host's
//! `tish_value_*` accessors, resolved at dlopen (`-undefined dynamic_lookup`). Every value is
//! deep-copied across the boundary.

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_void};

use gen::{Value, VmRef};

type H = *mut c_void;

const TAG_NULL: i32 = 0;
const TAG_NUMBER: i32 = 1;
const TAG_STRING: i32 = 2;
const TAG_BOOL: i32 = 3;
const TAG_ARRAY: i32 = 4;
const TAG_OBJECT: i32 = 5;

extern "C" {
    fn tish_value_new_number(n: f64) -> H;
    fn tish_value_new_bool(b: bool) -> H;
    fn tish_value_new_null() -> H;
    fn tish_value_new_string(s: *const c_char) -> H;
    fn tish_value_tag(r: H) -> i32;
    fn tish_value_as_number(r: H) -> f64;
    fn tish_value_as_bool(r: H) -> bool;
    fn tish_value_as_string(r: H) -> *mut c_char;
    fn tish_string_free(s: *mut c_char);
    fn tish_value_array_new() -> H;
    fn tish_value_array_push(arr: H, elem: H);
    fn tish_value_array_len(arr: H) -> usize;
    fn tish_value_array_get(arr: H, i: usize) -> H;
    fn tish_value_object_new() -> H;
    fn tish_value_object_set(obj: H, key: *const c_char, val: H);
    fn tish_value_drop(r: H);
}

#[repr(C)]
pub struct TishExport {
    pub name: *const c_char,
    pub func: extern "C" fn(*const H, usize) -> H,
}

#[repr(C)]
pub struct TishExportTable {
    pub exports: *const TishExport,
    pub count: usize,
}

/// Host handle -> plugin-local value. ABI v1 has no object-key accessor, so host objects
/// cannot be enumerated and arrive as null.
unsafe fn from_host(h: H) -> Value {
    match tish_value_tag(h) {
        TAG_NUMBER => Value::Number(tish_value_as_number(h)),
        TAG_BOOL => Value::Bool(tish_value_as_bool(h)),
        TAG_STRING => {
            let p = tish_value_as_string(h);
            if p.is_null() {
                return Value::Null;
            }
            let s = CStr::from_ptr(p).to_string_lossy().into_owned();
            tish_string_free(p);
            Value::String(s.into())
        }
        TAG_ARRAY => {
            let n = tish_value_array_len(h);
            let mut out = Vec::with_capacity(n);
            for i in 0..n {
                let e = tish_value_array_get(h, i);
                out.push(from_host(e));
                tish_value_drop(e);
            }
            Value::Array(VmRef::new(out))
        }
        TAG_OBJECT | TAG_NULL | _ => Value::Null,
    }
}

unsafe fn new_host_string(s: &str) -> H {
    match CString::new(s) {
        Ok(c) => tish_value_new_string(c.as_ptr()),
        Err(_) => tish_value_new_null(),
    }
}

/// Plugin-local value -> freshly owned host handle.
unsafe fn to_host(v: &Value) -> H {
    match v {
        Value::Null => tish_value_new_null(),
        Value::Number(n) => tish_value_new_number(*n),
        Value::Bool(b) => tish_value_new_bool(*b),
        Value::String(s) => new_host_string(&String::from_utf8_lossy(s.as_bytes())),
        Value::Array(a) => {
            let arr = tish_value_array_new();
            for e in a.borrow().iter() {
                let h = to_host(e);
                tish_value_array_push(arr, h);
                tish_value_drop(h);
            }
            arr
        }
        Value::NumberArray(a) => {
            let arr = tish_value_array_new();
            for e in a.borrow().to_values() {
                let h = to_host(&e);
                tish_value_array_push(arr, h);
                tish_value_drop(h);
            }
            arr
        }
        other => {
            // Objects and typed structs: enumerate through the runtime's own Object.entries.
            match gen::runtime::object_entries(&[other.clone()]) {
                Value::Array(pairs) => {
                    let obj = tish_value_object_new();
                    for pair in pairs.borrow().iter() {
                        if let Value::Array(kv) = pair {
                            let kv = kv.borrow();
                            if let (Some(Value::String(k)), Some(val)) = (kv.first(), kv.get(1)) {
                                if let Ok(key) = CString::new(k.as_bytes()) {
                                    let h = to_host(val);
                                    tish_value_object_set(obj, key.as_ptr(), h);
                                    tish_value_drop(h);
                                }
                            }
                        }
                    }
                    obj
                }
                _ => tish_value_new_null(),
            }
        }
    }
}

/// v1 has no error channel: a throw comes back as `{ error: "<message>" }`.
unsafe fn finish(out: Value) -> H {
    let thrown = gen::tish_last_throw();
    if !matches!(thrown, Value::Null) {
        let obj = tish_value_object_new();
        let msg = match &thrown {
            Value::String(s) => String::from_utf8_lossy(s.as_bytes()).into_owned(),
            other => format!("{:?}", other),
        };
        let h = new_host_string(&msg);
        tish_value_object_set(obj, b"error\0".as_ptr() as *const c_char, h);
        tish_value_drop(h);
        return obj;
    }
    to_host(&out)
}

unsafe fn arg(args: *const H, argc: usize, i: usize) -> Value {
    if i < argc {
        from_host(*args.add(i))
    } else {
        Value::Null
    }
}

macro_rules! export0 {
    ($name:ident) => {
        pub(crate) extern "C" fn $name(_args: *const H, _argc: usize) -> H {
            unsafe { finish(gen::$name()) }
        }
    };
}
macro_rules! export1 {
    ($name:ident) => {
        pub(crate) extern "C" fn $name(args: *const H, argc: usize) -> H {
            unsafe { finish(gen::$name(arg(args, argc, 0))) }
        }
    };
}
macro_rules! export2 {
    ($name:ident) => {
        pub(crate) extern "C" fn $name(args: *const H, argc: usize) -> H {
            unsafe { finish(gen::$name(arg(args, argc, 0), arg(args, argc, 1))) }
        }
    };
}

mod shims {
    use super::*;
    export0!(manifest);
    export0!(stats);
    export0!(fail);
    export1!(greet);
    export2!(search);
}

#[no_mangle]
pub extern "C" fn tish_module_register() -> *const TishExportTable {
    let entries: Vec<(&'static [u8], extern "C" fn(*const H, usize) -> H)> = vec![
        (b"manifest\0", shims::manifest),
        (b"stats\0", shims::stats),
        (b"fail\0", shims::fail),
        (b"greet\0", shims::greet),
        (b"search\0", shims::search),
    ];
    let exports: Vec<TishExport> = entries
        .into_iter()
        .map(|(name, func)| TishExport {
            name: name.as_ptr() as *const c_char,
            func,
        })
        .collect();
    let exports = Box::leak(exports.into_boxed_slice());
    Box::leak(Box::new(TishExportTable {
        exports: exports.as_ptr(),
        count: exports.len(),
    }))
}
