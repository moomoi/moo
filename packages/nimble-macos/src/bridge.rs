//! Results from worker threads back to Tish. Callbacks stay on the main thread, held by id; a
//! worker posts plain Rust data, which becomes a `Value` and reaches the callback on the main queue
//! under the UI root (a callback may call `setState`, which re-renders synchronously).

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use dispatch2::DispatchQueue;
use tishlang_core::Value;
use tishlang_ui::runtime::{run_with_current_root, LEGACY_ROOT_ID};

thread_local! {
    static CALLBACKS: RefCell<HashMap<u64, Value>> = RefCell::new(HashMap::new());
}

static NEXT: AtomicU64 = AtomicU64::new(1);

/// Keep `cb` (main thread) and return its id. A non-function is held too, and ignored on delivery.
pub fn hold(cb: Option<Value>) -> u64 {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    if let Some(cb) = cb {
        CALLBACKS.with(|c| c.borrow_mut().insert(id, cb));
    }
    id
}

pub fn release(id: u64) {
    CALLBACKS.with(|c| c.borrow_mut().remove(&id));
}

/// Call callback `id` with `v` now (main thread). `last` releases it.
pub fn call(id: u64, v: Value, last: bool) {
    let cb = CALLBACKS.with(|c| if last { c.borrow_mut().remove(&id) } else { c.borrow().get(&id).cloned() });
    if let Some(Value::Function(f)) = cb {
        run_with_current_root(LEGACY_ROOT_ID, || {
            let _ = f.call(&[v]);
        });
    }
}

/// From any thread: on the main queue, turn `data` into a value with `make` and pass it to `id`.
pub fn post<T: Send + 'static>(id: u64, data: T, make: fn(T) -> Value, last: bool) {
    DispatchQueue::main().exec_async(move || call(id, make(data), last));
}

/// Run `f` on the main queue (from a worker thread).
pub fn on_main(f: impl FnOnce() + Send + 'static) {
    DispatchQueue::main().exec_async(f);
}
