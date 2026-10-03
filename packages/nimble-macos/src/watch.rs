//! FSEvents on the application folders keeps the app index live. Events are coalesced by the
//! stream latency and delivered on the main queue, where the index lives.

use std::ffi::c_void;
use std::ptr;
use std::sync::OnceLock;

use dispatch2::DispatchQueue;
use objc2::rc::Retained;
use objc2_foundation::{NSArray, NSString};

type FSEventStreamRef = *mut c_void;
type Callback = extern "C" fn(FSEventStreamRef, *mut c_void, usize, *mut c_void, *const u32, *const u64);

#[repr(C)]
struct FSEventStreamContext {
    version: isize,
    info: *mut c_void,
    retain: *const c_void,
    release: *const c_void,
    copy_description: *const c_void,
}

#[link(name = "CoreServices", kind = "framework")]
extern "C" {
    fn FSEventStreamCreate(
        alloc: *const c_void,
        callback: Callback,
        context: *const FSEventStreamContext,
        paths: *const c_void,
        since_when: u64,
        latency: f64,
        flags: u32,
    ) -> FSEventStreamRef;
    fn FSEventStreamSetDispatchQueue(stream: FSEventStreamRef, queue: *const c_void);
    fn FSEventStreamStart(stream: FSEventStreamRef) -> u8;
}

const SINCE_NOW: u64 = u64::MAX;
const LATENCY_S: f64 = 2.0;

static ON_CHANGE: OnceLock<fn()> = OnceLock::new();

extern "C" fn on_events(
    _s: FSEventStreamRef,
    _info: *mut c_void,
    _n: usize,
    _paths: *mut c_void,
    _flags: *const u32,
    _ids: *const u64,
) {
    if let Some(f) = ON_CHANGE.get() {
        f();
    }
}

/// Watch `dirs` (recursively); `on_change` runs on the main queue after each coalesced batch.
pub fn watch(dirs: &[String], on_change: fn()) -> bool {
    let existing: Vec<Retained<NSString>> = dirs
        .iter()
        .filter(|d| std::path::Path::new(d).is_dir())
        .map(|d| NSString::from_str(d))
        .collect();
    if existing.is_empty() {
        return false;
    }
    let refs: Vec<&NSString> = existing.iter().map(|s| &**s).collect();
    let paths = NSArray::from_slice(&refs);
    if ON_CHANGE.set(on_change).is_err() {
        return false;
    }
    unsafe {
        let ctx = FSEventStreamContext {
            version: 0,
            info: ptr::null_mut(),
            retain: ptr::null(),
            release: ptr::null(),
            copy_description: ptr::null(),
        };
        let stream = FSEventStreamCreate(
            ptr::null(),
            on_events,
            &ctx,
            Retained::as_ptr(&paths) as *const c_void,
            SINCE_NOW,
            LATENCY_S,
            0,
        );
        if stream.is_null() {
            return false;
        }
        FSEventStreamSetDispatchQueue(stream, DispatchQueue::main() as *const DispatchQueue as *const c_void);
        FSEventStreamStart(stream) != 0
    }
}
