//! The file index as a service. A background thread loads the snapshot (or crawls when there is no
//! valid one) at utility QoS with throttled I/O; FSEvents then keeps it live from the snapshot's
//! event id, so changes made while Nimble was not running are replayed from the system journal.
//! Event handling runs on its own serial queue; searches run on the main thread and never wait
//! for it (a search that finds the index busy reports "not ready" and the shell falls back).

use std::ffi::{c_char, c_void, CStr};
use std::path::PathBuf;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant};

use dispatch2::{DispatchQueue, DispatchRetained};
use objc2::rc::Retained;
use objc2_foundation::{NSArray, NSString};

use crate::fsindex::{default_roots, FileHit, Index};

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
    fn FSEventStreamSetExclusionPaths(stream: FSEventStreamRef, paths: *const c_void) -> u8;
    fn FSEventStreamStart(stream: FSEventStreamRef) -> u8;
    fn FSEventsGetCurrentEventId() -> u64;
}

extern "C" {
    fn pthread_set_qos_class_self_np(qos: u32, priority: i32) -> i32;
    fn setiopolicy_np(iotype: i32, scope: i32, policy: i32) -> i32;
}

const QOS_CLASS_UTILITY: u32 = 0x11;
const IOPOL_TYPE_DISK: i32 = 0;
const IOPOL_SCOPE_THREAD: i32 = 1;
const IOPOL_THROTTLE: i32 = 3;

const MUST_SCAN_SUBDIRS: u32 = 0x1;
const USER_DROPPED: u32 = 0x2;
const KERNEL_DROPPED: u32 = 0x4;
const HISTORY_DONE: u32 = 0x10;
const ROOT_CHANGED: u32 = 0x20;

/// Coalescing window for change events.
const LATENCY_S: f64 = 1.0;
/// Save the snapshot at most this often while changes keep coming.
const SAVE_EVERY: Duration = Duration::from_secs(60);
/// Compact (drop removed entries) before saving once this share of entries is dead.
const COMPACT_AT: f64 = 0.2;

static INDEX: RwLock<Option<Index>> = RwLock::new(None);
static STARTED: AtomicBool = AtomicBool::new(false);
static STATUS: Mutex<Status> = Mutex::new(Status::new());
static QUEUE: OnceLock<DispatchRetained<DispatchQueue>> = OnceLock::new();
static SAVE: Mutex<SaveState> = Mutex::new(SaveState { last: None, dirty: false });

struct SaveState {
    last: Option<Instant>,
    dirty: bool,
}

#[derive(Clone, Debug)]
pub struct Status {
    /// "idle", "loading", "crawling", "ready"
    pub state: &'static str,
    pub entries: usize,
    pub dirs: usize,
    pub bytes: usize,
    pub build_ms: f64,
    pub from_snapshot: bool,
    /// Batches of changes that altered the index.
    pub updates: u64,
    /// Change events delivered by FSEvents, relevant or not.
    pub events: u64,
}

impl Status {
    const fn new() -> Status {
        Status { state: "idle", entries: 0, dirs: 0, bytes: 0, build_ms: 0.0, from_snapshot: false, updates: 0, events: 0 }
    }
}

pub fn status() -> Status {
    STATUS.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

fn set_state(state: &'static str) {
    STATUS.lock().unwrap_or_else(|e| e.into_inner()).state = state;
}

fn refresh_counts(ix: &Index) {
    let mut s = STATUS.lock().unwrap_or_else(|e| e.into_inner());
    s.entries = ix.len();
    s.dirs = ix.dir_count();
    s.bytes = ix.bytes();
}

pub fn snapshot_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("NIMBLE_FILE_INDEX") {
        return Some(PathBuf::from(p));
    }
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join("Library/Caches/Nimble/files.idx"))
}

/// Start the index (once). Returns immediately; `status().state` turns "ready" when searchable.
pub fn start() -> bool {
    if STARTED.swap(true, Ordering::AcqRel) {
        return true;
    }
    std::thread::Builder::new().name("nimble-fsindex".into()).spawn(build).is_ok()
}

fn background_thread() {
    unsafe {
        pthread_set_qos_class_self_np(QOS_CLASS_UTILITY, 0);
        setiopolicy_np(IOPOL_TYPE_DISK, IOPOL_SCOPE_THREAD, IOPOL_THROTTLE);
    }
}

fn crawl_now() -> Index {
    let since = unsafe { FSEventsGetCurrentEventId() };
    let (mut ix, stats) = Index::crawl(default_roots());
    ix.event_id = since;
    let mut s = STATUS.lock().unwrap_or_else(|e| e.into_inner());
    s.build_ms = stats.ms;
    s.entries = stats.entries;
    s.from_snapshot = false;
    ix
}

fn build() {
    background_thread();
    let roots = default_roots();
    if roots.is_empty() {
        return;
    }
    let path = snapshot_path();
    let current = unsafe { FSEventsGetCurrentEventId() };
    set_state("loading");
    let t0 = Instant::now();
    let loaded = path.as_ref().and_then(|p| Index::load(p, &roots)).filter(|ix| ix.event_id != 0 && ix.event_id <= current);
    let ix = match loaded {
        Some(ix) => {
            let mut s = STATUS.lock().unwrap_or_else(|e| e.into_inner());
            s.build_ms = t0.elapsed().as_secs_f64() * 1000.0;
            s.from_snapshot = true;
            ix
        }
        None => {
            set_state("crawling");
            let ix = crawl_now();
            if let Some(p) = &path {
                if let Err(e) = ix.save(p) {
                    eprintln!("nimble: file index save failed: {e}");
                }
            }
            SAVE.lock().unwrap_or_else(|e| e.into_inner()).last = Some(Instant::now());
            ix
        }
    };
    let since = ix.event_id;
    let watch: Vec<String> = ix.roots.iter().map(|r| r.path.clone()).collect();
    let exclude: Vec<String> =
        ix.roots.iter().flat_map(|r| r.skip_top.iter().map(move |s| format!("{}/{}", r.path, s))).take(MAX_EXCLUSIONS).collect();
    refresh_counts(&ix);
    *INDEX.write().unwrap_or_else(|e| e.into_inner()) = Some(ix);
    set_state("ready");
    if !watch_roots(&watch, &exclude, since) {
        eprintln!("nimble: file index is not watching for changes");
    }
}

/// FSEvents' limit on exclusion paths per stream.
const MAX_EXCLUSIONS: usize = 8;

fn ns_array(items: &[String]) -> Retained<NSArray<NSString>> {
    let ns: Vec<Retained<NSString>> = items.iter().map(|r| NSString::from_str(r)).collect();
    let refs: Vec<&NSString> = ns.iter().map(|s| &**s).collect();
    NSArray::from_slice(&refs)
}

/// Watch `roots` from event `since`; changes under `exclude` (e.g. ~/Library, which churns
/// constantly) are filtered by the kernel rather than delivered and discarded.
fn watch_roots(roots: &[String], exclude: &[String], since: u64) -> bool {
    let paths = ns_array(roots);
    let excluded = ns_array(exclude);
    let queue = QUEUE.get_or_init(|| DispatchQueue::new("nimble.fsindex", None));
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
            since,
            LATENCY_S,
            0,
        );
        if stream.is_null() {
            return false;
        }
        if !exclude.is_empty() && FSEventStreamSetExclusionPaths(stream, Retained::as_ptr(&excluded) as *const c_void) == 0 {
            eprintln!("nimble: file index could not exclude {exclude:?}");
        }
        FSEventStreamSetDispatchQueue(stream, &**queue as *const DispatchQueue as *const c_void);
        FSEventStreamStart(stream) != 0
    }
}

/// Runs on the index's serial queue; the only place the index changes after it is built.
extern "C" fn on_events(_s: FSEventStreamRef, _info: *mut c_void, n: usize, paths: *mut c_void, flags: *const u32, ids: *const u64) {
    let paths = paths as *const *const c_char;
    let mut shallow = Vec::new();
    let mut deep = Vec::new();
    let mut full = false;
    let mut last = 0u64;
    STATUS.lock().unwrap_or_else(|e| e.into_inner()).events += n as u64;
    for i in 0..n {
        let (f, id) = unsafe { (*flags.add(i), *ids.add(i)) };
        last = last.max(id);
        if f & HISTORY_DONE != 0 {
            continue;
        }
        let p = unsafe { CStr::from_ptr(*paths.add(i)) }.to_string_lossy().trim_end_matches('/').to_string();
        if f & ROOT_CHANGED != 0 {
            full = true;
        } else if f & (MUST_SCAN_SUBDIRS | USER_DROPPED | KERNEL_DROPPED) != 0 {
            deep.push(p);
        } else {
            shallow.push(p);
        }
    }
    background_thread();
    let roots: Vec<String> = match INDEX.read().unwrap_or_else(|e| e.into_inner()).as_ref() {
        Some(ix) => {
            shallow.retain(|p| ix.covers(p));
            deep.retain(|p| ix.covers(p));
            ix.roots.iter().map(|r| r.path.clone()).collect()
        }
        None => return,
    };
    full |= deep.iter().any(|p| roots.contains(p));
    if full {
        let mut fresh = crawl_now();
        fresh.event_id = fresh.event_id.max(last);
        refresh_counts(&fresh);
        *INDEX.write().unwrap_or_else(|e| e.into_inner()) = Some(fresh);
        mark_dirty();
    } else if !shallow.is_empty() || !deep.is_empty() {
        let mut guard = INDEX.write().unwrap_or_else(|e| e.into_inner());
        if let Some(ix) = guard.as_mut() {
            let mut changed = ix.rescan(&deep, true);
            changed += ix.rescan(&shallow, false);
            ix.event_id = ix.event_id.max(last);
            if changed > 0 {
                refresh_counts(ix);
                STATUS.lock().unwrap_or_else(|e| e.into_inner()).updates += 1;
            }
        }
        drop(guard);
        mark_dirty();
    } else if let Some(ix) = INDEX.write().unwrap_or_else(|e| e.into_inner()).as_mut() {
        ix.event_id = ix.event_id.max(last);
    }
    maybe_save();
}

fn mark_dirty() {
    SAVE.lock().unwrap_or_else(|e| e.into_inner()).dirty = true;
}

/// Save when dirty and the last save is older than SAVE_EVERY; compacts a copy first if needed,
/// so searches keep running on the live index meanwhile.
fn maybe_save() {
    let due = {
        let s = SAVE.lock().unwrap_or_else(|e| e.into_inner());
        s.dirty && s.last.is_none_or(|t| t.elapsed() >= SAVE_EVERY)
    };
    let Some(path) = snapshot_path().filter(|_| due) else { return };
    let needs_compact = INDEX.read().unwrap_or_else(|e| e.into_inner()).as_ref().is_some_and(|ix| ix.dead_ratio() > COMPACT_AT);
    if needs_compact {
        let copy = INDEX.read().unwrap_or_else(|e| e.into_inner()).as_ref().map(Index::clone);
        if let Some(mut copy) = copy {
            copy.compact();
            refresh_counts(&copy);
            *INDEX.write().unwrap_or_else(|e| e.into_inner()) = Some(copy);
        }
    }
    let result = INDEX.read().unwrap_or_else(|e| e.into_inner()).as_ref().map(|ix| ix.save(&path));
    if let Some(Err(e)) = result {
        eprintln!("nimble: file index save failed: {e}");
    }
    let mut s = SAVE.lock().unwrap_or_else(|e| e.into_inner());
    s.dirty = false;
    s.last = Some(Instant::now());
}

/// Ranked matches, or `None` while the index is building or briefly busy applying changes.
pub fn search(query: &str, limit: usize) -> Option<(Vec<FileHit>, f64)> {
    let guard = INDEX.try_read().ok()?;
    let ix = guard.as_ref()?;
    let t0 = Instant::now();
    let hits = ix.search(query, limit, |p| crate::frecency::boost(p) as i32 * 8);
    Some((hits, t0.elapsed().as_secs_f64() * 1000.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait_until(what: &str, secs: u64, f: impl Fn() -> bool) {
        let t0 = Instant::now();
        while !f() {
            assert!(t0.elapsed() < Duration::from_secs(secs), "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn names(q: &str) -> Vec<String> {
        loop {
            if let Some((hits, _)) = search(q, 20) {
                return hits.into_iter().map(|h| h.name).collect();
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Real FSEvents on a temporary root: build, then pick up a created file, a created folder
    /// with contents, and deletions. Owns the process-wide index, so it runs alone (ignored).
    #[test]
    #[ignore]
    fn live_index_follows_changes() {
        let root = std::env::temp_dir().join(format!("nimble-fslive-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("docs")).unwrap();
        std::fs::write(root.join("docs/quarterly-report.txt"), "x").unwrap();
        let snap = root.with_extension("idx");
        std::env::set_var("NIMBLE_FILE_ROOTS", &root);
        std::env::set_var("NIMBLE_FILE_INDEX", &snap);

        assert!(start());
        wait_until("ready", 10, || status().state == "ready");
        assert_eq!(names("quarterly"), vec!["quarterly-report.txt"]);
        assert!(snap.exists(), "first build saves a snapshot");

        std::fs::write(root.join("docs/zebra-notes.md"), "x").unwrap();
        wait_until("new file", 10, || names("zebra") == vec!["zebra-notes.md"]);

        std::fs::create_dir_all(root.join("moved/inner")).unwrap();
        std::fs::write(root.join("moved/inner/walrus.pdf"), "x").unwrap();
        wait_until("new folder contents", 10, || names("walrus") == vec!["walrus.pdf"]);

        std::fs::remove_dir_all(root.join("moved")).unwrap();
        std::fs::remove_file(root.join("docs/zebra-notes.md")).unwrap();
        wait_until("deletions", 10, || names("walrus").is_empty() && names("zebra").is_empty());
        assert!(status().updates >= 3, "updates: {}", status().updates);

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_file(&snap);
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct TimeVal {
        sec: i64,
        usec: i32,
    }

    #[repr(C)]
    struct RUsage {
        utime: TimeVal,
        stime: TimeVal,
        /// `ru_maxrss` (bytes on macOS) first, then the other counters.
        rest: [i64; 14],
    }

    extern "C" {
        fn getrusage(who: i32, usage: *mut RUsage) -> i32;
    }

    /// (user + system CPU ms, max RSS MB) for this process.
    fn rusage() -> (f64, f64) {
        let mut u = RUsage { utime: TimeVal { sec: 0, usec: 0 }, stime: TimeVal { sec: 0, usec: 0 }, rest: [0; 14] };
        unsafe { getrusage(0, &mut u) };
        let cpu = |t: TimeVal| t.sec as f64 * 1000.0 + t.usec as f64 / 1000.0;
        (cpu(u.utime) + cpu(u.stime), u.rest[0] as f64 / 1048576.0)
    }

    /// `NIMBLE_FILE_INDEX=/tmp/x.idx cargo test real_home_live -- --ignored --nocapture`, twice:
    /// the first run crawls, the second loads the snapshot.
    #[test]
    #[ignore]
    fn real_home_live() {
        let (cpu0, rss0) = rusage();
        let t0 = Instant::now();
        start();
        wait_until("ready", 120, || status().state == "ready");
        let s = status();
        let (cpu1, rss1) = rusage();
        eprintln!(
            "ready in {:.0} ms ({}), {} entries, {:.1} MB index, cpu {:.0} ms, max rss {:.0} -> {:.0} MB",
            t0.elapsed().as_secs_f64() * 1000.0,
            if s.from_snapshot { "snapshot" } else { "crawl" },
            s.entries,
            s.bytes as f64 / 1048576.0,
            cpu1 - cpu0,
            rss0,
            rss1
        );
        for q in ["re", "readme", "invoice", "main.tish", "nimble docs", "scrnsht", "png"] {
            let (hits, ms) = search(q, 8).unwrap();
            eprintln!("{q:>12}: {ms:.2} ms  {}", hits.first().map_or("-", |h| h.path.as_str()));
        }
        let (cpu1, _) = rusage();
        std::thread::sleep(Duration::from_secs(10));
        let (cpu2, _) = rusage();
        let s = status();
        eprintln!("idle 10 s: cpu {:.1} ms, events {}, updates {}", cpu2 - cpu1, s.events, s.updates);
    }
}

/// Files and folders opened through Nimble, most used first, that still exist.
pub fn recent(limit: usize) -> Vec<FileHit> {
    let guard = INDEX.read().unwrap_or_else(|e| e.into_inner());
    let Some(ix) = guard.as_ref() else { return Vec::new() };
    crate::frecency::ranked(limit * 2, |k| k.starts_with('/') && !k.ends_with(".app") && ix.covers(k))
        .into_iter()
        .filter_map(|p| {
            let meta = std::fs::symlink_metadata(&p).ok()?;
            let name = p.rsplit('/').next()?.to_string();
            Some(FileHit { detail: ix.detail_for_path(&p), name, is_dir: meta.is_dir(), score: 0, path: p })
        })
        .take(limit)
        .collect()
}
