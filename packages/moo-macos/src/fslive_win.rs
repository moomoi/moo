//! Windows: the file index as a service, with fslive.rs's API. A background thread loads the
//! snapshot so search is ready at once, then crawls again (Windows keeps no change journal Moo can
//! replay, so the snapshot may be stale) and swaps the fresh index in. ReadDirectoryChangesW (the
//! `notify` crate) keeps it live: changed folders are rescanned, coalesced over a second. Paths in
//! the index are `/`-separated (`C:/Users/me/...`), which Windows accepts.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Mutex, RwLock};
use std::time::{Duration, Instant};

use notify::{RecursiveMode, Watcher};

use crate::fsindex::{default_roots, FileHit, Index};

const LATENCY: Duration = Duration::from_secs(1);
const SAVE_EVERY: Duration = Duration::from_secs(60);
const COMPACT_AT: f64 = 0.2;

static INDEX: RwLock<Option<Index>> = RwLock::new(None);
static STARTED: AtomicBool = AtomicBool::new(false);
static STATUS: Mutex<Status> = Mutex::new(Status::new());

#[derive(Clone, Debug)]
pub struct Status {
    /// "idle", "loading", "crawling", "ready"
    pub state: &'static str,
    pub entries: usize,
    pub dirs: usize,
    pub bytes: usize,
    pub build_ms: f64,
    pub from_snapshot: bool,
    pub updates: u64,
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
    if let Some(p) = std::env::var_os("MOO_FILE_INDEX") {
        return Some(PathBuf::from(p));
    }
    Some(PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("Moo").join("files.idx"))
}

/// Start the index (once). Returns immediately; `status().state` turns "ready" when searchable.
pub fn start() -> bool {
    if STARTED.swap(true, Ordering::AcqRel) {
        return true;
    }
    std::thread::Builder::new().name("moo-fsindex".into()).spawn(build).is_ok()
}

fn save(ix: &Index) {
    if let Some(p) = snapshot_path() {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = ix.save(&p) {
            eprintln!("moo: file index save failed: {e}");
        }
    }
}

fn build() {
    let roots = default_roots();
    if roots.is_empty() {
        return;
    }
    set_state("loading");
    let t0 = Instant::now();
    if let Some(ix) = snapshot_path().and_then(|p| Index::load(&p, &roots)) {
        {
            let mut s = STATUS.lock().unwrap_or_else(|e| e.into_inner());
            s.build_ms = t0.elapsed().as_secs_f64() * 1000.0;
            s.from_snapshot = true;
        }
        refresh_counts(&ix);
        *INDEX.write().unwrap_or_else(|e| e.into_inner()) = Some(ix);
        set_state("ready");
    } else {
        set_state("crawling");
    }
    // Watch first, so nothing that changes during the crawl is missed.
    let (tx, rx) = mpsc::channel();
    let mut watcher = match notify::recommended_watcher(move |r: notify::Result<notify::Event>| {
        if let Ok(ev) = r {
            let _ = tx.send(ev.paths);
        }
    }) {
        Ok(w) => Some(w),
        Err(e) => {
            eprintln!("moo: file index is not watching for changes: {e}");
            None
        }
    };
    if let Some(w) = watcher.as_mut() {
        for r in &roots {
            if let Err(e) = w.watch(std::path::Path::new(&r.path), RecursiveMode::Recursive) {
                eprintln!("moo: can't watch {}: {e}", r.path);
            }
        }
    }
    let (fresh, stats) = Index::crawl(roots);
    {
        let mut s = STATUS.lock().unwrap_or_else(|e| e.into_inner());
        s.build_ms = stats.ms;
        s.entries = stats.entries;
        s.from_snapshot = false;
    }
    refresh_counts(&fresh);
    save(&fresh);
    *INDEX.write().unwrap_or_else(|e| e.into_inner()) = Some(fresh);
    set_state("ready");
    live(rx);
    drop(watcher);
}

/// Apply changes as they come: the folders holding changed paths are rescanned, in batches.
fn live(rx: mpsc::Receiver<Vec<PathBuf>>) {
    let mut last_save = Instant::now();
    let mut dirty = false;
    while let Ok(first) = rx.recv() {
        let mut changed: BTreeSet<String> = BTreeSet::new();
        let mut add = |paths: Vec<PathBuf>| {
            for p in paths {
                let s = p.to_string_lossy().replace('\\', "/");
                // The folder holding the change: a shallow rescan sees adds, removes and renames.
                let dir = s.rsplit_once('/').map_or(s.clone(), |(d, _)| d.to_string());
                changed.insert(dir);
            }
        };
        add(first);
        let until = Instant::now() + LATENCY;
        while let Ok(more) = rx.recv_timeout(until.saturating_duration_since(Instant::now())) {
            add(more);
        }
        STATUS.lock().unwrap_or_else(|e| e.into_inner()).events += changed.len() as u64;
        let mut guard = INDEX.write().unwrap_or_else(|e| e.into_inner());
        if let Some(ix) = guard.as_mut() {
            let dirs: Vec<String> = changed.into_iter().filter(|d| ix.covers(d)).collect();
            if !dirs.is_empty() && ix.rescan(&dirs, false) > 0 {
                refresh_counts(ix);
                STATUS.lock().unwrap_or_else(|e| e.into_inner()).updates += 1;
                dirty = true;
            }
            if dirty && last_save.elapsed() >= SAVE_EVERY {
                if ix.dead_ratio() > COMPACT_AT {
                    ix.compact();
                }
                save(ix);
                dirty = false;
                last_save = Instant::now();
            }
        }
    }
}

/// Ranked matches, or `None` while the index is building or briefly busy applying changes.
pub fn search(query: &str, limit: usize) -> Option<(Vec<FileHit>, f64)> {
    let guard = INDEX.try_read().ok()?;
    let ix = guard.as_ref()?;
    let t0 = Instant::now();
    let hits = ix.search(query, limit, |p| crate::frecency::boost(p) as i32 * 8);
    Some((hits, t0.elapsed().as_secs_f64() * 1000.0))
}

/// Files and folders opened through Moo, most used first, that still exist.
pub fn recent(limit: usize) -> Vec<FileHit> {
    let guard = INDEX.read().unwrap_or_else(|e| e.into_inner());
    let Some(ix) = guard.as_ref() else { return Vec::new() };
    crate::frecency::ranked(limit * 2, |k| k.as_bytes().get(1) == Some(&b':') && ix.covers(k))
        .into_iter()
        .filter_map(|p| {
            let meta = std::fs::symlink_metadata(&p).ok()?;
            let name = p.rsplit('/').next()?.to_string();
            Some(FileHit { detail: ix.detail_for_path(&p), name, is_dir: meta.is_dir(), score: 0, path: p })
        })
        .take(limit)
        .collect()
}
