//! Apple's on-device model through `swift/ai.swift`. Replies stream on a Swift concurrency thread;
//! events are queued here and delivered to the Tish callback on the main queue. Consecutive partial
//! replies for a request collapse into the newest, since each one carries the whole text so far.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{c_char, CStr, CString};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use dispatch2::DispatchQueue;
use tishlang_core::Value;
type Callback = extern "C" fn(u64, i32, *const c_char);
type ToolCallback = extern "C" fn(u64, *const c_char, *const c_char) -> *mut c_char;

extern "C" {
    fn moo_ai_availability() -> *mut c_char;
    fn moo_ai_free(p: *mut c_char);
    fn moo_ai_session_new(instructions: *const c_char, tools: *const c_char, tool_cb: Option<ToolCallback>) -> u64;
    fn strdup(s: *const c_char) -> *mut c_char;
    fn moo_ai_session_free(id: u64);
    fn moo_ai_prewarm(id: u64);
    fn moo_ai_ask(session: u64, request: u64, prompt: *const c_char, cb: Callback) -> bool;
    fn moo_ai_cancel(request: u64);
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

/// Tests have no main dispatch loop, so tool calls are recorded instead of run.
#[cfg(test)]
static TOOL_CALLS_RECORDED: AtomicBool = AtomicBool::new(false);
#[cfg(test)]
static TOOL_CALLS: Mutex<Vec<(u64, String, String)>> = Mutex::new(Vec::new());

thread_local! {
    static CALLBACKS: RefCell<HashMap<u64, Value>> = RefCell::new(HashMap::new());
    /// Per session: the Tish `(name, argsJson) -> text` handler for its tools (main thread).
    static TOOL_HANDLERS: RefCell<HashMap<u64, Value>> = RefCell::new(HashMap::new());
}

/// `Ok(())` when the model can answer, else the reason (e.g. `appleIntelligenceNotEnabled`).
pub fn availability() -> Result<(), String> {
    let s = unsafe {
        let p = moo_ai_availability();
        let s = CStr::from_ptr(p).to_string_lossy().into_owned();
        moo_ai_free(p);
        s
    };
    if s == "available" { Ok(()) } else { Err(s) }
}

/// `tools_json`: `[{ name, description, params: [{ name, description, optional?, choices? }] }]`
/// (string parameters). When the model calls a
/// tool, `on_tool(name, argsJson)` runs on the main thread and its return value (as text) goes back
/// to the model.
pub fn session(instructions: &str, tools_json: &str, on_tool: Option<Value>) -> u64 {
    let c = CString::new(instructions.replace('\0', "")).unwrap();
    let t = CString::new(tools_json.replace('\0', "")).unwrap();
    let cb: Option<ToolCallback> = if on_tool.is_some() { Some(on_swift_tool) } else { None };
    let id = unsafe { moo_ai_session_new(c.as_ptr(), t.as_ptr(), cb) };
    if let (true, Some(f)) = (id != 0, on_tool) {
        TOOL_HANDLERS.with(|h| h.borrow_mut().insert(id, f));
    }
    id
}

pub fn end_session(id: u64) {
    unsafe { moo_ai_session_free(id) }
    TOOL_HANDLERS.with(|h| h.borrow_mut().remove(&id));
}

/// Runs on a Swift concurrency thread, never the main thread, so waiting on the main queue is safe.
extern "C" fn on_swift_tool(session: u64, name: *const c_char, args: *const c_char) -> *mut c_char {
    let read = |p: *const c_char| if p.is_null() { String::new() } else { unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned() };
    let (name, args) = (read(name), read(args));
    #[cfg(test)]
    if TOOL_CALLS_RECORDED.load(Ordering::Acquire) {
        TOOL_CALLS.lock().unwrap().push((session, name, args));
        return unsafe { strdup(c"Done.".as_ptr()) };
    }
    let out = Arc::new(Mutex::new(String::new()));
    let slot = out.clone();
    let call = format!("{name} {args}");
    DispatchQueue::main().exec_sync(move || {
        let reply = run_tool(session, &name, &args);
        *slot.lock().unwrap() = reply;
    });
    let reply = std::mem::take(&mut *out.lock().unwrap());
    eprintln!("moo: AI tool {call} -> {}", reply.lines().next().unwrap_or(""));
    let c = CString::new(reply.replace('\0', "")).unwrap();
    unsafe { strdup(c.as_ptr()) }
}

fn run_tool(session: u64, name: &str, args: &str) -> String {
    let Some(Value::Function(f)) = TOOL_HANDLERS.with(|h| h.borrow().get(&session).cloned()) else {
        return format!("Tool {name} is not available.");
    };
    crate::mac::with_ui(|| {
        let r = f.call(&[Value::String(name.into()), Value::String(args.into())]);
        match tishlang_core::take_pending_throw() {
            Some(e) => format!("Tool {name} failed: {}", error_text(&e)),
            None => r.to_display_string(),
        }
    })
}

fn error_text(e: &Value) -> String {
    match e {
        Value::Object(o) => o.borrow().strings.get("message").map(|m| m.to_display_string()).unwrap_or_default(),
        other => other.to_display_string(),
    }
}

pub fn prewarm(id: u64) {
    unsafe { moo_ai_prewarm(id) }
}

/// Start a streamed reply; `on_event` gets `(kind, text)` on the main thread. Returns the request id,
/// or 0 if the session is unknown or still answering.
pub fn ask(session: u64, prompt: &str, on_event: Value) -> u64 {
    let request = NEXT_REQUEST.fetch_add(1, Ordering::Relaxed);
    CALLBACKS.with(|c| c.borrow_mut().insert(request, on_event));
    let c = CString::new(prompt.replace('\0', "")).unwrap();
    if unsafe { moo_ai_ask(session, request, c.as_ptr(), on_swift_event) } {
        request
    } else {
        CALLBACKS.with(|c| c.borrow_mut().remove(&request));
        0
    }
}

pub fn cancel(request: u64) {
    unsafe { moo_ai_cancel(request) }
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
    crate::mac::with_ui(|| {
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

    /// Both tests use the global event queue.
    static QUEUE: Mutex<()> = Mutex::new(());

    #[test]
    fn partials_for_one_request_collapse_to_the_newest() {
        let _q = QUEUE.lock().unwrap_or_else(|e| e.into_inner());
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

    /// The real model, asked to open something, calls the host's `open` tool with a target.
    /// Skipped when Apple Intelligence is off.
    #[test]
    fn model_calls_a_host_tool() {
        let _q = QUEUE.lock().unwrap_or_else(|e| e.into_inner());
        if availability().is_err() {
            eprintln!("skip: Apple Intelligence unavailable");
            return;
        }
        TOOL_CALLS_RECORDED.store(true, Ordering::Release);
        FLUSH_QUEUED.store(true, Ordering::Release);
        let tools = r#"[{"name":"open","description":"Open an application, file, folder or web address on this Mac.","params":[{"name":"target","description":"An app name such as Safari, a path such as ~/Documents, or a URL"}]}]"#;
        let home = std::env::var("HOME").unwrap_or_default();
        let instructions = format!(
            "You are the assistant inside a macOS launcher. When the user asks to open something, call the open tool. The user's home folder is {home}."
        );
        let s = session(&instructions, tools, Some(Value::Null));
        assert!(s > 0);
        let t0 = std::time::Instant::now();
        let req = ask(s, "open finder to my documents", Value::Null);
        assert!(req > 0);
        let mut reply = None;
        while reply.is_none() && t0.elapsed() < std::time::Duration::from_secs(60) {
            std::thread::sleep(std::time::Duration::from_millis(100));
            reply = EVENTS.lock().unwrap().iter().find(|e| e.0 == req && e.1 != Kind::Partial).map(|e| (e.1, e.2.clone()));
        }
        let (kind, text) = reply.expect("no reply within 60 s");
        let calls: Vec<_> = TOOL_CALLS.lock().unwrap().iter().filter(|c| c.0 == s).cloned().collect();
        eprintln!("{:.1} s, {kind:?}: {text}\ncalls: {calls:?}", t0.elapsed().as_secs_f64());
        end_session(s);
        EVENTS.lock().unwrap().clear();
        FLUSH_QUEUED.store(false, Ordering::Release);
        assert_eq!(kind, Kind::Done, "{text}");
        assert!(calls.iter().any(|c| c.1 == "open" && c.2.contains("Documents")), "the model never called open: {calls:?}");
    }

    /// Talks to the real model when this Mac has Apple Intelligence on; otherwise checks the
    /// unavailable path reports a reason and refuses to open a session.
    #[test]
    fn availability_matches_session_creation() {
        match availability() {
            Ok(()) => {
                let s = session("Reply with one word.", "[]", None);
                assert!(s > 0);
                end_session(s);
            }
            Err(reason) => {
                assert!(!reason.is_empty());
                assert_eq!(session("", "[]", None), 0);
            }
        }
    }
}
