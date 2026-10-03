//! Apple's on-device model through `swift/ai.swift`. Replies stream on a Swift concurrency thread;
//! events are queued here and delivered to the Tish callback on the main queue. Consecutive partial
//! replies for a request collapse into the newest, since each one carries the whole text so far.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{c_char, CStr, CString};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

use dispatch2::DispatchQueue;
use tishlang_core::Value;
use tishlang_ui::runtime::{run_with_current_root, LEGACY_ROOT_ID};

type Callback = extern "C" fn(u64, i32, *const c_char);

extern "C" {
    fn nimble_ai_availability() -> *mut c_char;
    fn nimble_ai_free(p: *mut c_char);
    fn nimble_ai_session_new(instructions: *const c_char) -> u64;
    fn nimble_ai_session_free(id: u64);
    fn nimble_ai_prewarm(id: u64);
    fn nimble_ai_ask(session: u64, request: u64, prompt: *const c_char, cb: Callback) -> bool;
    fn nimble_ai_cancel(request: u64);
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    Partial,
    Done,
    Error,
    Cancelled,
}

impl Kind {
    fn from_raw(k: i32) -> Kind {
        match k {
            0 => Kind::Partial,
            1 => Kind::Done,
            3 => Kind::Cancelled,
            _ => Kind::Error,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Kind::Partial => "partial",
            Kind::Done => "done",
            Kind::Error => "error",
            Kind::Cancelled => "cancelled",
        }
    }
}

static EVENTS: Mutex<Vec<(u64, Kind, String)>> = Mutex::new(Vec::new());
static FLUSH_QUEUED: AtomicBool = AtomicBool::new(false);
static NEXT_REQUEST: AtomicU64 = AtomicU64::new(1);

thread_local! {
    static CALLBACKS: RefCell<HashMap<u64, Value>> = RefCell::new(HashMap::new());
}

/// `Ok(())` when the model can answer, else the reason (e.g. `appleIntelligenceNotEnabled`).
pub fn availability() -> Result<(), String> {
    let s = unsafe {
        let p = nimble_ai_availability();
        let s = CStr::from_ptr(p).to_string_lossy().into_owned();
        nimble_ai_free(p);
        s
    };
    if s == "available" { Ok(()) } else { Err(s) }
}

pub fn session(instructions: &str) -> u64 {
    let c = CString::new(instructions.replace('\0', "")).unwrap();
    unsafe { nimble_ai_session_new(c.as_ptr()) }
}

pub fn end_session(id: u64) {
    unsafe { nimble_ai_session_free(id) }
}

pub fn prewarm(id: u64) {
    unsafe { nimble_ai_prewarm(id) }
}

/// Start a streamed reply; `on_event` gets `(kind, text)` on the main thread. Returns the request id,
/// or 0 if the session is unknown or still answering.
pub fn ask(session: u64, prompt: &str, on_event: Value) -> u64 {
    let request = NEXT_REQUEST.fetch_add(1, Ordering::Relaxed);
    CALLBACKS.with(|c| c.borrow_mut().insert(request, on_event));
    let c = CString::new(prompt.replace('\0', "")).unwrap();
    if unsafe { nimble_ai_ask(session, request, c.as_ptr(), on_swift_event) } {
        request
    } else {
        CALLBACKS.with(|c| c.borrow_mut().remove(&request));
        0
    }
}

pub fn cancel(request: u64) {
    unsafe { nimble_ai_cancel(request) }
}

extern "C" fn on_swift_event(request: u64, kind: i32, text: *const c_char) {
    let text = if text.is_null() { String::new() } else { unsafe { CStr::from_ptr(text) }.to_string_lossy().into_owned() };
    queue_event(request, Kind::from_raw(kind), text);
}

fn queue_event(request: u64, kind: Kind, text: String) {
    {
        let mut q = EVENTS.lock().unwrap();
        match q.last_mut() {
            Some(last) if last.0 == request && last.1 == Kind::Partial && kind == Kind::Partial => last.2 = text,
            _ => q.push((request, kind, text)),
        }
    }
    if !FLUSH_QUEUED.swap(true, Ordering::AcqRel) {
        DispatchQueue::main().exec_async(flush);
    }
}

fn flush() {
    FLUSH_QUEUED.store(false, Ordering::Release);
    let events = std::mem::take(&mut *EVENTS.lock().unwrap());
    if events.is_empty() {
        return;
    }
    run_with_current_root(LEGACY_ROOT_ID, || {
        for (request, kind, text) in events {
            let cb = CALLBACKS.with(|c| {
                let mut c = c.borrow_mut();
                if kind == Kind::Partial { c.get(&request).cloned() } else { c.remove(&request) }
            });
            if let Some(Value::Function(f)) = cb {
                let payload = crate::obj(vec![
                    ("request", Value::Number(request as f64)),
                    ("kind", Value::String(kind.name().into())),
                    ("text", Value::String(text.as_str().into())),
                ]);
                let _ = f.call(&[payload]);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partials_for_one_request_collapse_to_the_newest() {
        FLUSH_QUEUED.store(true, Ordering::Release);
        EVENTS.lock().unwrap().clear();
        queue_event(7, Kind::Partial, "a".into());
        queue_event(7, Kind::Partial, "ab".into());
        queue_event(8, Kind::Partial, "x".into());
        queue_event(7, Kind::Partial, "abc".into());
        queue_event(7, Kind::Done, "abc".into());
        let q = std::mem::take(&mut *EVENTS.lock().unwrap());
        let got: Vec<_> = q.iter().map(|(r, k, t)| (*r, *k, t.as_str())).collect();
        assert_eq!(
            got,
            vec![(7, Kind::Partial, "ab"), (8, Kind::Partial, "x"), (7, Kind::Partial, "abc"), (7, Kind::Done, "abc")]
        );
        FLUSH_QUEUED.store(false, Ordering::Release);
    }

    /// Talks to the real model when this Mac has Apple Intelligence on; otherwise checks the
    /// unavailable path reports a reason and refuses to open a session.
    #[test]
    fn availability_matches_session_creation() {
        match availability() {
            Ok(()) => {
                let s = session("Reply with one word.");
                assert!(s > 0);
                end_session(s);
            }
            Err(reason) => {
                assert!(!reason.is_empty());
                assert_eq!(session(""), 0);
            }
        }
    }
}
