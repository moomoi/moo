//! FSEvents on the application folders keeps the app index live. Events are coalesced by the
//! stream latency and delivered on the main queue, where the index lives.

use std::ffi::c_void;
use std::ptr;

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

extern "C" fn on_events(
    _s: FSEventStreamRef,
    info: *mut c_void,
    _n: usize,
    _paths: *mut c_void,
    _flags: *const u32,
    _ids: *const u64,
) {
    if !info.is_null() {
        let f: fn() = unsafe { std::mem::transmute::<*mut c_void, fn()>(info) };
        f();
    }
}

/// Watch `dirs` (recursively); `on_change` runs on the main queue after each coalesced batch.
pub fn watch(dirs: &[String], on_change: fn()) -> bool {
    watch_with_latency(dirs, on_change, LATENCY_S)
}

/// As `watch`, coalescing over `latency` seconds. Each call adds a stream.
pub fn watch_with_latency(dirs: &[String], on_change: fn(), latency: f64) -> bool {
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
    unsafe {
        let ctx = FSEventStreamContext {
            version: 0,
            info: on_change as *mut c_void,
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
            latency,
            0,
        );
        if stream.is_null() {
            return false;
        }
        FSEventStreamSetDispatchQueue(stream, DispatchQueue::main() as *const DispatchQueue as *const c_void);
        FSEventStreamStart(stream) != 0
    }
}
