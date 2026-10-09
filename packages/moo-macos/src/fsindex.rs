//! Moo's own file name index: every name under the roots in memory, so a search is a scan of
//! compact arrays (a few milliseconds) instead of a Spotlight round trip. Portable: no AppKit.
//!
//! Layout: one entry per file or folder, stored as columns (`parent`, `name_off`, `name_len`,
//! `flags`, `depth`, `mask`) plus one byte buffer of names, deduplicated when built or compacted.
//! About 16 bytes per entry plus name bytes. `mask` has one bit per letter (and digit bucket) in the
//! name, so most entries are rejected without touching their name.
//!
//! What is left out, to keep the index small and the results useful: hidden entries, `~/Library`
//! (iCloud Drive is added as its own root), dependency and build trees (`node_modules`, `target`,
//! ...), the insides of packages such as `.app` and `.photoslibrary`, and the contents of "bulk"
//! folders below the top level that hold more than [`BULK_LIMIT`] entries (machine-generated data;
//! the folder itself stays searchable).
//!
//! Changes arrive as folder paths (from FSEvents); [`Index::rescan`] diffs each folder's children
//! against the disk, crawls new subfolders and drops vanished ones. A snapshot ([`Index::save`])
//! plus the last FSEvents id lets a restart replay the system's change journal instead of crawling.

use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime};

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

const NONE: u32 = u32::MAX;
pub const DIR: u8 = 1;
/// Opaque bundle (`.app`, `.photoslibrary`, ...): listed, never descended.
pub const PACKAGE: u8 = 2;
pub const DEAD: u8 = 4;
/// Folder listed, contents skipped (more than BULK_LIMIT entries below the top level).
pub const BULK: u8 = 8;
/// Inside vendored, build or cache trees: still searchable, ranked below everything else.
pub const NOISE: u8 = 16;

const NOISE_DIRS: &[&str] = &[
    "third_party", "thirdparty", "third-party", "vendor", "vendors", "deps", "external", "externals", "extern",
    "site-packages", "dist", "build", "out", "obj", "Intermediate", "Binaries", "DerivedDataCache", "Library",
];
const NOISE_PENALTY: i32 = 250;

pub const BULK_LIMIT: usize = 5000;
/// Depth (below a root) from which the bulk rule applies; top-level folders such as Downloads are
/// always indexed in full.
const BULK_MIN_DEPTH: u8 = 2;

const SKIP_DIRS: &[&str] = &["node_modules", "target", "__pycache__", "Pods", "DerivedData", "bower_components"];
const PACKAGE_EXTS: &[&str] = &[
    "app", "appex", "bundle", "framework", "plugin", "kext", "xpc", "dsym", "docc", "photoslibrary",
    "musiclibrary", "tvlibrary", "imovielibrary", "fcpbundle", "logicx", "band", "rtfd", "pages",
    "numbers", "key", "xcodeproj", "xcworkspace", "playground", "scriptd",
];

const MAGIC: &[u8; 8] = b"NIMFIX02";

/// Below this many entries a search runs on the calling thread.
const PAR_MIN: usize = 100_000;
const MAX_THREADS: usize = 8;
/// Fuzzy candidates scored by nucleo per search (shortest names first).
const FUZZY_MAX: usize = 3000;

thread_local! {
    static FUZZY_MATCHER: std::cell::RefCell<Matcher> = std::cell::RefCell::new(Matcher::new(Config::DEFAULT));
}

#[derive(Clone, Debug)]
pub struct Root {
    pub path: String,
    /// Top-level names to skip (e.g. `Library` in the home folder).
    pub skip_top: Vec<String>,
    /// Shown instead of the path prefix in result details (`~`, `iCloud Drive`).
    pub label: String,
}

#[derive(Clone, Debug)]
pub struct FileHit {
    pub name: String,
    pub path: String,
    /// Parent folder for display, with the root's label and left-truncated.
    pub detail: String,
    pub is_dir: bool,
    pub score: i32,
}

#[derive(Default, Clone)]
pub struct Index {
    pub roots: Vec<Root>,
    parent: Vec<u32>,
    name_off: Vec<u32>,
    name_len: Vec<u16>,
    flags: Vec<u8>,
    depth: Vec<u8>,
    mask: Vec<u32>,
    names: Vec<u8>,
    /// Folder path hash -> entry, for applying change events.
    dirs: HashMap<u64, u32>,
    dead: usize,
    /// Last FSEvents id applied (0 when unknown).
    pub event_id: u64,
}

fn mask_of(bytes: &[u8]) -> u32 {
    let mut m = 0u32;
    for &b in bytes {
        let l = b.to_ascii_lowercase();
        if l.is_ascii_lowercase() {
            m |= 1 << (l - b'a');
        } else if l.is_ascii_digit() {
            m |= 1 << (26 + (l - b'0') % 6);
        }
    }
    m
}

fn path_hash(path: &str) -> u64 {
    let mut h = DefaultHasher::new();
    path.as_bytes().hash(&mut h);
    h.finish()
}

fn is_package(name: &str) -> bool {
    name.rsplit_once('.').is_some_and(|(_, ext)| PACKAGE_EXTS.iter().any(|p| p.eq_ignore_ascii_case(ext)))
}

/// Case-insensitive (ASCII) substring search; `needle` is already lowercase.
fn find_ci(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > hay.len() {
        return if needle.is_empty() { Some(0) } else { None };
    }
    let first = needle[0];
    'outer: for i in 0..=hay.len() - needle.len() {
        if hay[i].to_ascii_lowercase() != first {
            continue;
        }
        for j in 1..needle.len() {
            if hay[i + j].to_ascii_lowercase() != needle[j] {
                continue 'outer;
            }
        }
        return Some(i);
    }
    None
}

/// Case-insensitive subsequence test (fuzzy fallback).
fn subsequence_ci(hay: &[u8], needle: &[u8]) -> bool {
    let mut j = 0;
    for &b in hay {
        if j < needle.len() && b.to_ascii_lowercase() == needle[j] {
            j += 1;
        }
    }
    j == needle.len()
}

fn word_start(name: &[u8], pos: usize) -> bool {
    if pos == 0 {
        return true;
    }
    let prev = name[pos - 1];
    let cur = name[pos];
    matches!(prev, b' ' | b'-' | b'_' | b'.' | b'(' | b'[') || (prev.is_ascii_lowercase() && cur.is_ascii_uppercase())
}

/// One child as listed on disk, after the skip rules.
struct Listed {
    name: String,
    flags: u8,
}

enum Listing {
    Children(Vec<Listed>),
    Bulk,
    Unreadable,
}

/// List `dir`'s children under the skip rules. `depth` is the folder's own depth below its root.
fn list_dir(dir: &Path, depth: u8, root: &Root) -> Listing {
    let Ok(rd) = std::fs::read_dir(dir) else { return Listing::Unreadable };
    let mut out = Vec::new();
    let mut seen = 0usize;
    for entry in rd.flatten() {
        seen += 1;
        if depth >= BULK_MIN_DEPTH && seen > BULK_LIMIT {
            return Listing::Bulk;
        }
        let Ok(name) = entry.file_name().into_string() else { continue };
        if name.starts_with('.') || name.len() > u16::MAX as usize {
            continue;
        }
        let Ok(ft) = entry.file_type() else { continue };
        let flags = if ft.is_dir() {
            if SKIP_DIRS.contains(&name.as_str()) || (depth == 0 && root.skip_top.iter().any(|s| s == &name)) {
                continue;
            }
            if is_package(&name) { PACKAGE } else { DIR }
        } else {
            0
        };
        out.push(Listed { name, flags });
    }
    Listing::Children(out)
}

pub struct CrawlStats {
    pub entries: usize,
    pub ms: f64,
}

impl Index {
    pub fn new(roots: Vec<Root>) -> Index {
        let mut ix = Index { roots, ..Default::default() };
        for i in 0..ix.roots.len() {
            let path = ix.roots[i].path.clone();
            let id = ix.push(NONE, &path, DIR, 0, None);
            ix.dirs.insert(path_hash(&path), id);
        }
        ix
    }

    pub fn len(&self) -> usize {
        self.parent.len() - self.dead
    }

    pub fn dir_count(&self) -> usize {
        self.dirs.len()
    }

    /// Heap bytes held by the index.
    pub fn bytes(&self) -> usize {
        self.parent.capacity() * 4
            + self.name_off.capacity() * 4
            + self.name_len.capacity() * 2
            + self.flags.capacity()
            + self.depth.capacity()
            + self.mask.capacity() * 4
            + self.names.capacity()
            + self.dirs.capacity() * 16
    }

    fn push(&mut self, parent: u32, name: &str, flags: u8, depth: u8, interner: Option<&mut HashMap<Box<[u8]>, u32>>) -> u32 {
        let bytes = name.as_bytes();
        let off = match interner {
            Some(map) => match map.get(bytes) {
                Some(&off) => off,
                None => {
                    let off = self.names.len() as u32;
                    self.names.extend_from_slice(bytes);
                    map.insert(bytes.into(), off);
                    off
                }
            },
            None => {
                let off = self.names.len() as u32;
                self.names.extend_from_slice(bytes);
                off
            }
        };
        let id = self.parent.len() as u32;
        let inherited = parent != NONE && self.flags[parent as usize] & NOISE != 0;
        let noisy = inherited || (parent != NONE && flags & DIR != 0 && NOISE_DIRS.contains(&name));
        self.parent.push(parent);
        self.name_off.push(off);
        self.name_len.push(bytes.len() as u16);
        self.flags.push(if noisy { flags | NOISE } else { flags });
        self.depth.push(depth);
        self.mask.push(if parent == NONE { 0 } else { mask_of(bytes) });
        id
    }

    fn name(&self, id: u32) -> &[u8] {
        let o = self.name_off[id as usize] as usize;
        &self.names[o..o + self.name_len[id as usize] as usize]
    }

    fn name_str(&self, id: u32) -> &str {
        std::str::from_utf8(self.name(id)).unwrap_or("")
    }

    fn root_of(&self, mut id: u32) -> usize {
        while self.parent[id as usize] != NONE {
            id = self.parent[id as usize];
        }
        id as usize
    }

    pub fn path(&self, id: u32) -> String {
        let mut chain = Vec::new();
        let mut cur = id;
        while cur != NONE {
            chain.push(cur);
            cur = self.parent[cur as usize];
        }
        let mut s = String::new();
        for (i, c) in chain.iter().rev().enumerate() {
            if i > 0 {
                s.push('/');
            }
            s.push_str(self.name_str(*c));
        }
        s
    }

    /// Crawl `dir` (already an entry) and everything below it.
    fn crawl_from(&mut self, dir: u32, dir_path: PathBuf, interner: &mut Option<HashMap<Box<[u8]>, u32>>) {
        let root = self.roots[self.root_of(dir)].clone();
        let mut stack = vec![(dir, dir_path)];
        while let Some((id, path)) = stack.pop() {
            let depth = self.depth[id as usize];
            match list_dir(&path, depth, &root) {
                Listing::Children(children) => {
                    for c in children {
                        let child = self.push(id, &c.name, c.flags, depth.saturating_add(1), interner.as_mut());
                        if c.flags == DIR {
                            let p = path.join(&c.name);
                            self.dirs.insert(path_hash(&p.to_string_lossy()), child);
                            stack.push((child, p));
                        }
                    }
                }
                Listing::Bulk => self.flags[id as usize] |= BULK,
                Listing::Unreadable => {}
            }
        }
    }

    /// Build the whole index from disk.
    pub fn crawl(roots: Vec<Root>) -> (Index, CrawlStats) {
        let t0 = Instant::now();
        let mut ix = Index::new(roots);
        let mut interner = Some(HashMap::new());
        for r in 0..ix.roots.len() {
            let p = PathBuf::from(&ix.roots[r].path);
            ix.crawl_from(r as u32, p, &mut interner);
        }
        ix.shrink();
        let stats = CrawlStats { entries: ix.len(), ms: t0.elapsed().as_secs_f64() * 1000.0 };
        (ix, stats)
    }

    fn shrink(&mut self) {
        self.parent.shrink_to_fit();
        self.name_off.shrink_to_fit();
        self.name_len.shrink_to_fit();
        self.flags.shrink_to_fit();
        self.depth.shrink_to_fit();
        self.mask.shrink_to_fit();
        self.names.shrink_to_fit();
        self.dirs.shrink_to_fit();
    }

    // ── Search ──────────────────────────────────────────────────────────────

    fn is_live_item(&self, id: usize) -> bool {
        self.flags[id] & DEAD == 0 && self.parent[id] != NONE
    }

    /// Whether `atom` occurs in a folder name above `id` (roots excluded).
    fn in_ancestors(&self, id: u32, atom: &[u8]) -> bool {
        let mut cur = self.parent[id as usize];
        while cur != NONE && self.parent[cur as usize] != NONE {
            if find_ci(self.name(cur), atom).is_some() {
                return true;
            }
            cur = self.parent[cur as usize];
        }
        false
    }

    fn base_score(&self, id: u32, pos: usize, atom_len: usize) -> i32 {
        let name = self.name(id);
        let nl = name.len();
        let stem = name.iter().rposition(|&b| b == b'.').filter(|&i| i > 0).unwrap_or(nl);
        let mut s = 1000;
        if nl == atom_len {
            s += 400;
        } else if pos == 0 && stem == atom_len {
            s += 370;
        }
        if self.flags[id as usize] & NOISE != 0 {
            s -= NOISE_PENALTY;
        }
        if pos == 0 {
            s += 150;
        } else if word_start(name, pos) {
            s += 90;
        }
        s -= ((nl - atom_len).min(60) * 3) as i32;
        s -= self.depth[id as usize] as i32 * 12;
        if self.flags[id as usize] & (DIR | PACKAGE) != 0 {
            s += 20;
        }
        s
    }

    /// Ranked matches. Every word of `query` must occur in the name or a parent folder's name, and
    /// at least one in the name; with one word and few substring matches, a fuzzy (subsequence)
    /// pass fills in. `boost(path)` adds frecency; recently modified files rank higher.
    pub fn search(&self, query: &str, limit: usize, boost: impl Fn(&str) -> i32) -> Vec<FileHit> {
        let atoms: Vec<Vec<u8>> = query.split_whitespace().map(|a| a.to_lowercase().into_bytes()).collect();
        if atoms.is_empty() || limit == 0 {
            return Vec::new();
        }
        let masks: Vec<u32> = atoms.iter().map(|a| mask_of(a)).collect();
        let mut scored = self.par_scan(|range| self.scan_words(range, &atoms, &masks));
        if atoms.len() == 1 && scored.len() < limit && atoms[0].len() >= 3 {
            let have: HashSet<u32> = scored.iter().map(|x| x.1).collect();
            let fuzzy = self.par_scan(|range| self.scan_fuzzy(range, &atoms[0], masks[0], &have));
            scored.extend(self.score_fuzzy(fuzzy, query.trim()));
        }
        self.finish(scored, limit, boost)
    }

    /// Run `scan` over the entries, split across cores when the index is large.
    fn par_scan(&self, scan: impl Fn(std::ops::Range<usize>) -> Vec<(i32, u32)> + Sync) -> Vec<(i32, u32)> {
        let n = self.parent.len();
        let threads = if n < PAR_MIN { 1 } else { std::thread::available_parallelism().map_or(1, |p| p.get()).min(MAX_THREADS) };
        if threads <= 1 {
            return scan(0..n);
        }
        let chunk = n.div_ceil(threads);
        std::thread::scope(|s| {
            let handles: Vec<_> = (0..threads)
                .map(|t| {
                    let scan = &scan;
                    s.spawn(move || scan(t * chunk..((t + 1) * chunk).min(n)))
                })
                .collect();
            handles.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
        })
    }

    fn scan_fuzzy(&self, range: std::ops::Range<usize>, atom: &[u8], mask: u32, have: &HashSet<u32>) -> Vec<(i32, u32)> {
        range
            .filter(|&id| self.is_live_item(id) && mask & !self.mask[id] == 0 && !have.contains(&(id as u32)))
            .filter(|&id| subsequence_ci(self.name(id as u32), atom))
            .map(|id| (0, id as u32))
            .collect()
    }

    /// Fuzzy candidates scored by nucleo (word starts and runs count), below substring matches.
    fn score_fuzzy(&self, mut cands: Vec<(i32, u32)>, query: &str) -> Vec<(i32, u32)> {
        if cands.len() > FUZZY_MAX {
            cands.select_nth_unstable_by_key(FUZZY_MAX - 1, |c| self.name_len[c.1 as usize]);
            cands.truncate(FUZZY_MAX);
        }
        let pattern = Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);
        let mut buf = Vec::new();
        FUZZY_MATCHER.with(|m| {
            let mut m = m.borrow_mut();
            cands
                .into_iter()
                .filter_map(|(_, id)| {
                    let score = pattern.score(Utf32Str::new(self.name_str(id), &mut buf), &mut m)? as i32;
                    let noise = if self.flags[id as usize] & NOISE != 0 { NOISE_PENALTY } else { 0 };
                    Some((300 + score - self.depth[id as usize] as i32 * 12 - noise, id))
                })
                .collect()
        })
    }

    fn scan_words(&self, range: std::ops::Range<usize>, atoms: &[Vec<u8>], masks: &[u32]) -> Vec<(i32, u32)> {
        let mut scored = Vec::new();
        for id in range {
            if !self.is_live_item(id) {
                continue;
            }
            let m = self.mask[id];
            if masks.iter().all(|am| am & !m != 0) {
                continue;
            }
            let name = self.name(id as u32);
            let mut best: Option<(usize, usize)> = None; // (atom, pos) matched in the name
            let mut in_name = 0;
            for (k, atom) in atoms.iter().enumerate() {
                if masks[k] & !m != 0 {
                    continue;
                }
                if let Some(pos) = find_ci(name, atom) {
                    in_name += 1;
                    if best.is_none_or(|(bk, _)| atoms[bk].len() < atom.len()) {
                        best = Some((k, pos));
                    }
                }
            }
            let Some((bk, pos)) = best else { continue };
            let mut s = self.base_score(id as u32, pos, atoms[bk].len());
            if atoms.len() > 1 {
                let mut ok = true;
                for (k, atom) in atoms.iter().enumerate() {
                    if k == bk || find_ci(name, atom).is_some() {
                        continue;
                    }
                    if self.in_ancestors(id as u32, atom) {
                        s += 15;
                    } else {
                        ok = false;
                        break;
                    }
                }
                if !ok {
                    continue;
                }
                s += (in_name as i32 - 1) * 40;
            }
            scored.push((s, id as u32));
        }
        scored
    }

    /// Rerank a pool with what needs the path (frecency, modification time) and build the hits.
    fn finish(&self, mut scored: Vec<(i32, u32)>, limit: usize, boost: impl Fn(&str) -> i32) -> Vec<FileHit> {
        let pool = limit.max(64).min(scored.len());
        if scored.len() > pool {
            scored.select_nth_unstable_by(pool - 1, |a, b| b.0.cmp(&a.0));
            scored.truncate(pool);
        }
        let now = SystemTime::now();
        let mut hits: Vec<FileHit> = scored
            .into_iter()
            .map(|(s, id)| {
                let path = self.path(id);
                let age_days = std::fs::symlink_metadata(&path)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| now.duration_since(t).ok())
                    .map_or(f64::MAX, |d| d.as_secs_f64() / 86_400.0);
                let recency = if age_days < 1.0 {
                    120
                } else if age_days < 7.0 {
                    80
                } else if age_days < 30.0 {
                    40
                } else if age_days < 365.0 {
                    10
                } else {
                    0
                };
                let score = s + recency + boost(&path);
                FileHit {
                    name: self.name_str(id).to_string(),
                    detail: self.detail(id),
                    is_dir: self.flags[id as usize] & DIR != 0,
                    path,
                    score,
                }
            })
            .collect();
        hits.sort_by(|a, b| b.score.cmp(&a.score).then(a.path.len().cmp(&b.path.len())));
        hits.truncate(limit);
        hits
    }

    fn detail(&self, id: u32) -> String {
        self.detail_for_path(&self.path(id))
    }

    /// Parent folder for display: the root's label, then the path below it, left-truncated.
    pub fn detail_for_path(&self, path: &str) -> String {
        const MAX: usize = 40;
        let parent = path.rsplit_once('/').map_or("", |(p, _)| p);
        let root = self
            .roots
            .iter()
            .filter(|r| parent == r.path || parent.starts_with(&format!("{}/", r.path)))
            .max_by_key(|r| r.path.len());
        let short = match root {
            Some(r) => format!("{}{}", r.label, &parent[r.path.len()..]),
            None => parent.to_string(),
        };
        let n = short.chars().count();
        if n <= MAX {
            short
        } else {
            format!("…{}", short.chars().skip(n - (MAX - 1)).collect::<String>())
        }
    }

    // ── Updates ─────────────────────────────────────────────────────────────

    /// The folder entry for an absolute path, if indexed.
    fn dir_id(&self, path: &str) -> Option<u32> {
        let id = *self.dirs.get(&path_hash(path))?;
        (self.flags[id as usize] & DEAD == 0 && self.path(id) == path).then_some(id)
    }

    /// Whether `path` is somewhere the index covers (inside a root, not skipped by the rules).
    pub fn covers(&self, path: &str) -> bool {
        self.roots.iter().any(|r| {
            let Some(rest) = path.strip_prefix(&r.path) else { return false };
            if !(rest.is_empty() || rest.starts_with('/')) {
                return false;
            }
            let mut parts = rest.split('/').filter(|p| !p.is_empty());
            if let Some(first) = parts.clone().next() {
                if r.skip_top.iter().any(|s| s == first) {
                    return false;
                }
            }
            parts.all(|p| !p.starts_with('.') && !SKIP_DIRS.contains(&p) && !is_package(p))
        })
    }

    /// Indexed folder for `path`, or the nearest indexed ancestor (a new folder's parent).
    fn nearest_dir(&self, path: &str) -> Option<u32> {
        let mut p = path.trim_end_matches('/');
        loop {
            if let Some(id) = self.dir_id(p) {
                return Some(id);
            }
            p = &p[..p.rfind('/')?];
            if p.is_empty() {
                return None;
            }
        }
    }

    fn kill_subtree(&mut self, top: u32) {
        self.kill_subtrees(&[top]);
    }

    /// Kill every entry under `tops`, and the tops, in one pass over the index. Entries are only
    /// appended, so a child's id is always greater than its parent's: walking forward from the
    /// lowest top, an entry is doomed when its parent is. (This used to scan the whole index once
    /// per level of every removed subtree, about N × entries for deleting a folder.)
    fn kill_subtrees(&mut self, tops: &[u32]) {
        let Some(&start) = tops.iter().min() else { return };
        let start = start as usize;
        let mut doomed = vec![false; self.parent.len() - start];
        for &t in tops {
            doomed[t as usize - start] = true;
            self.kill(t);
        }
        for id in start + 1..self.parent.len() {
            let parent = self.parent[id];
            if parent == NONE || (parent as usize) < start {
                continue;
            }
            if !doomed[id - start] && doomed[parent as usize - start] {
                doomed[id - start] = true;
                self.kill(id as u32);
            }
        }
    }

    fn kill(&mut self, id: u32) {
        let f = &mut self.flags[id as usize];
        if *f & DEAD == 0 {
            *f |= DEAD;
            self.dead += 1;
            if *f & DIR != 0 {
                let p = self.path(id);
                self.dirs.remove(&path_hash(&p));
            }
        }
    }

    /// Bring folders in line with the disk. `recursive`: rebuild everything below each folder
    /// (FSEvents "must scan subdirs"); otherwise only direct children are compared, and new
    /// subfolders are crawled. Returns the number of entries added plus removed.
    pub fn rescan(&mut self, paths: &[String], recursive: bool) -> usize {
        let before = (self.parent.len(), self.dead);
        let mut targets: Vec<u32> = paths.iter().filter(|p| self.covers(p)).filter_map(|p| self.nearest_dir(p)).collect();
        targets.sort_unstable();
        targets.dedup();
        if targets.is_empty() {
            return 0;
        }
        let set: HashSet<u32> = targets.iter().copied().collect();
        let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
        for id in 0..self.parent.len() {
            if self.flags[id] & DEAD == 0 && set.contains(&self.parent[id]) {
                children.entry(self.parent[id]).or_default().push(id as u32);
            }
        }
        let mut interner = None;
        for dir in targets {
            if self.flags[dir as usize] & DEAD != 0 {
                continue;
            }
            let dir_path = self.path(dir);
            let existing = children.remove(&dir).unwrap_or_default();
            if recursive {
                self.kill_subtrees(&existing);
                self.flags[dir as usize] &= !BULK;
                self.crawl_from(dir, PathBuf::from(&dir_path), &mut interner);
                continue;
            }
            let root = self.roots[self.root_of(dir)].clone();
            let depth = self.depth[dir as usize];
            match list_dir(Path::new(&dir_path), depth, &root) {
                Listing::Children(listed) => {
                    self.flags[dir as usize] &= !BULK;
                    let on_disk: HashMap<&str, u8> = listed.iter().map(|l| (l.name.as_str(), l.flags)).collect();
                    let mut present: HashSet<String> = HashSet::new();
                    let mut gone = Vec::new();
                    for c in existing {
                        let name = self.name_str(c).to_string();
                        let keep = on_disk.get(name.as_str()) == Some(&(self.flags[c as usize] & (DIR | PACKAGE)));
                        if keep {
                            present.insert(name);
                        } else {
                            gone.push(c);
                        }
                    }
                    self.kill_subtrees(&gone);
                    for l in listed {
                        if present.contains(&l.name) {
                            continue;
                        }
                        let child = self.push(dir, &l.name, l.flags, depth.saturating_add(1), None);
                        if l.flags == DIR {
                            let p = PathBuf::from(&dir_path).join(&l.name);
                            self.dirs.insert(path_hash(&p.to_string_lossy()), child);
                            self.crawl_from(child, p, &mut interner);
                        }
                    }
                }
                Listing::Bulk => {
                    self.kill_subtrees(&existing);
                    self.flags[dir as usize] |= BULK;
                }
                Listing::Unreadable => {
                    if !Path::new(&dir_path).exists() {
                        self.kill_subtree(dir);
                    }
                }
            }
        }
        (self.parent.len() - before.0) + (self.dead - before.1)
    }

    pub fn dead_ratio(&self) -> f64 {
        if self.parent.is_empty() { 0.0 } else { self.dead as f64 / self.parent.len() as f64 }
    }

    /// Drop dead entries and deduplicate names.
    pub fn compact(&mut self) {
        let mut out = Index { roots: self.roots.clone(), event_id: self.event_id, ..Default::default() };
        let mut remap = vec![NONE; self.parent.len()];
        let mut interner = HashMap::new();
        for id in 0..self.parent.len() {
            if self.flags[id] & DEAD != 0 {
                continue;
            }
            let parent = self.parent[id];
            let new_parent = if parent == NONE { NONE } else { remap[parent as usize] };
            if parent != NONE && new_parent == NONE {
                continue;
            }
            let name = self.name_str(id as u32).to_string();
            let nid = out.push(new_parent, &name, self.flags[id], self.depth[id], Some(&mut interner));
            remap[id] = nid;
        }
        for id in 0..out.parent.len() {
            if out.flags[id] & DIR != 0 {
                let p = out.path(id as u32);
                out.dirs.insert(path_hash(&p), id as u32);
            }
        }
        out.shrink();
        *self = out;
    }

    // ── Snapshot ────────────────────────────────────────────────────────────

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("tmp");
        let mut w = std::io::BufWriter::new(std::fs::File::create(&tmp)?);
        w.write_all(MAGIC)?;
        w.write_all(&self.event_id.to_le_bytes())?;
        w.write_all(&(self.roots.len() as u32).to_le_bytes())?;
        for r in &self.roots {
            for s in [&r.path, &r.label] {
                w.write_all(&(s.len() as u32).to_le_bytes())?;
                w.write_all(s.as_bytes())?;
            }
            w.write_all(&(r.skip_top.len() as u32).to_le_bytes())?;
            for s in &r.skip_top {
                w.write_all(&(s.len() as u32).to_le_bytes())?;
                w.write_all(s.as_bytes())?;
            }
        }
        let n = self.parent.len();
        w.write_all(&(n as u32).to_le_bytes())?;
        w.write_all(&(self.names.len() as u32).to_le_bytes())?;
        for v in &self.parent {
            w.write_all(&v.to_le_bytes())?;
        }
        for v in &self.name_off {
            w.write_all(&v.to_le_bytes())?;
        }
        for v in &self.name_len {
            w.write_all(&v.to_le_bytes())?;
        }
        w.write_all(&self.flags)?;
        w.write_all(&self.depth)?;
        for v in &self.mask {
            w.write_all(&v.to_le_bytes())?;
        }
        w.write_all(&self.names)?;
        w.flush()?;
        drop(w);
        std::fs::rename(tmp, path)
    }

    /// Load a snapshot; `None` if missing, corrupt, or built for different roots.
    pub fn load(path: &Path, roots: &[Root]) -> Option<Index> {
        let mut bytes = Vec::new();
        std::fs::File::open(path).ok()?.read_to_end(&mut bytes).ok()?;
        let mut r = Reader { b: &bytes, at: 0 };
        if r.take(8)? != MAGIC {
            return None;
        }
        let event_id = r.u64()?;
        let nroots = r.u32()? as usize;
        let mut saved = Vec::new();
        for _ in 0..nroots {
            let path = r.string()?;
            let label = r.string()?;
            let nskip = r.u32()? as usize;
            let mut skip_top = Vec::new();
            for _ in 0..nskip {
                skip_top.push(r.string()?);
            }
            saved.push(Root { path, label, skip_top });
        }
        let same = saved.len() == roots.len()
            && saved.iter().zip(roots).all(|(a, b)| a.path == b.path && a.skip_top == b.skip_top);
        if !same {
            return None;
        }
        let n = r.u32()? as usize;
        let names_len = r.u32()? as usize;
        let mut ix = Index { roots: roots.to_vec(), event_id, ..Default::default() };
        ix.parent = r.u32s(n)?;
        ix.name_off = r.u32s(n)?;
        ix.name_len = (0..n).map(|_| r.u16()).collect::<Option<_>>()?;
        ix.flags = r.take(n)?.to_vec();
        ix.depth = r.take(n)?.to_vec();
        ix.mask = r.u32s(n)?;
        ix.names = r.take(names_len)?.to_vec();
        for id in 0..n {
            let (o, l) = (ix.name_off[id] as usize, ix.name_len[id] as usize);
            if o + l > names_len || (ix.parent[id] != NONE && ix.parent[id] as usize >= id) {
                return None;
            }
        }
        for id in 0..n {
            if ix.flags[id] & DEAD != 0 {
                ix.dead += 1;
            } else if ix.flags[id] & DIR != 0 {
                let p = ix.path(id as u32);
                ix.dirs.insert(path_hash(&p), id as u32);
            }
        }
        ix.dirs.shrink_to_fit();
        Some(ix)
    }
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Option<&[u8]> {
        let s = self.b.get(self.at..self.at.checked_add(n)?)?;
        self.at += n;
        Some(s)
    }
    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
    fn u32s(&mut self, n: usize) -> Option<Vec<u32>> {
        let raw = self.take(n.checked_mul(4)?)?;
        Some(raw.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect())
    }
    fn string(&mut self) -> Option<String> {
        let n = self.u32()? as usize;
        String::from_utf8(self.take(n)?.to_vec()).ok()
    }
}

/// The home folder (without `~/Library`) and iCloud Drive when present.
/// Home (without ~/Library) plus iCloud Drive. `MOO_FILE_ROOTS` (colon-separated) replaces them.
pub fn default_roots() -> Vec<Root> {
    if let Some(list) = std::env::var_os("MOO_FILE_ROOTS") {
        return std::env::split_paths(&list)
            .filter_map(|p| p.canonicalize().ok())
            .map(|p| {
                let path = p.to_string_lossy().into_owned();
                let label = p.file_name().map_or(path.clone(), |n| n.to_string_lossy().into_owned());
                Root { path, skip_top: Vec::new(), label }
            })
            .collect();
    }
    // Windows: the user profile without AppData (its ~/Library), as a `/`-separated path: the index
    // joins and splits on `/`, and Windows takes either separator.
    #[cfg(windows)]
    {
        let Some(home) = std::env::var_os("USERPROFILE").map(|h| h.to_string_lossy().replace('\\', "/")) else { return Vec::new() };
        vec![Root { path: home, skip_top: vec!["AppData".into()], label: "~".into() }]
    }
    #[cfg(not(windows))]
    let Some(home) = std::env::var_os("HOME").map(|h| h.to_string_lossy().into_owned()) else { return Vec::new() };
    #[cfg(not(windows))]
    let mut roots = vec![Root { path: home.clone(), skip_top: vec!["Library".into()], label: "~".into() }];
    #[cfg(not(windows))]
    {
        let icloud = format!("{home}/Library/Mobile Documents/com~apple~CloudDocs");
        if Path::new(&icloud).is_dir() {
            roots.push(Root { path: icloud, skip_top: Vec::new(), label: "iCloud Drive".into() });
        }
        roots
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempTree(PathBuf);

    impl TempTree {
        fn new(tag: &str) -> TempTree {
            let p = std::env::temp_dir().join(format!("moo-fsindex-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            TempTree(p)
        }
        fn file(&self, rel: &str) {
            let p = self.0.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, b"x").unwrap();
        }
        fn root(&self) -> Root {
            Root { path: self.0.to_string_lossy().into_owned(), skip_top: vec!["Library".into()], label: "~".into() }
        }
    }

    impl Drop for TempTree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn names(hits: &[FileHit]) -> Vec<&str> {
        hits.iter().map(|h| h.name.as_str()).collect()
    }

    #[test]
    fn crawl_applies_the_skip_rules() {
        let t = TempTree::new("skip");
        t.file("Documents/report.pdf");
        t.file("Documents/.secret");
        t.file(".hidden/notes.txt");
        t.file("Library/Caches/junk.db");
        t.file("code/app/node_modules/left-pad/index.js");
        t.file("code/app/target/debug/app");
        t.file("code/app/src/main.rs");
        t.file("Apps/Thing.app/Contents/Info.plist");
        let (ix, stats) = Index::crawl(vec![t.root()]);
        let mut all: Vec<String> = (0..ix.parent.len() as u32).filter(|&i| ix.is_live_item(i as usize)).map(|i| ix.name_str(i).to_string()).collect();
        all.sort();
        assert_eq!(all, ["Apps", "Documents", "Thing.app", "app", "code", "main.rs", "report.pdf", "src"], "{all:?}");
        assert_eq!(stats.entries, ix.len());
    }

    #[test]
    fn bulk_folders_keep_the_folder_but_not_its_contents() {
        let t = TempTree::new("bulk");
        for i in 0..(BULK_LIMIT + 5) {
            t.file(&format!("proj/data/blocks/{i:06}.bin"));
        }
        t.file("proj/data/readme.md");
        let (ix, _) = Index::crawl(vec![t.root()]);
        assert_eq!(names(&ix.search("blocks", 5, |_| 0)), ["blocks"]);
        assert!(ix.search("000001", 5, |_| 0).is_empty());
        assert_eq!(names(&ix.search("readme", 5, |_| 0)), ["readme.md"]);
    }

    #[test]
    fn ranking_prefers_exact_then_prefix_then_shallow() {
        let t = TempTree::new("rank");
        t.file("notes.md");
        t.file("deep/a/b/c/notes.md");
        t.file("old-notes-backup.txt");
        t.file("Projects/notesapp/readme.md");
        let (ix, _) = Index::crawl(vec![t.root()]);
        let hits = ix.search("notes", 10, |_| 0);
        assert_eq!(hits[0].path, t.0.join("notes.md").to_string_lossy());
        assert_eq!(names(&hits)[1], "notes.md");
        assert!(hits.iter().position(|h| h.name == "notesapp").unwrap() < hits.iter().position(|h| h.name == "old-notes-backup.txt").unwrap());
    }

    #[test]
    fn extra_words_can_match_parent_folders() {
        let t = TempTree::new("words");
        t.file("Projects/moo/app/main.tish");
        t.file("Projects/other/main.tish");
        let (ix, _) = Index::crawl(vec![t.root()]);
        let hits = ix.search("moo main", 10, |_| 0);
        assert_eq!(hits.len(), 1, "{:?}", names(&hits));
        assert!(hits[0].path.ends_with("moo/app/main.tish"));
        assert_eq!(hits[0].detail, "~/Projects/moo/app");
    }

    #[test]
    fn fuzzy_fills_in_when_substrings_are_scarce() {
        let t = TempTree::new("fuzzy");
        t.file("quarterly-report.pdf");
        let (ix, _) = Index::crawl(vec![t.root()]);
        assert_eq!(names(&ix.search("qrtrep", 5, |_| 0)), ["quarterly-report.pdf"]);
    }

    /// Deleting a big folder removes every entry under it in one pass, leaves siblings (and
    /// entries added after it) alone, and stays fast.
    #[test]
    fn removing_a_large_folder_is_one_pass() {
        let t = TempTree::new("bigdelete");
        for i in 0..400 {
            t.file(&format!("Projects/big/sub{}/file{i}.txt", i % 20));
        }
        t.file("Projects/keep/stay.txt");
        let (mut ix, _) = Index::crawl(vec![t.root()]);
        t.file("Projects/later/after.txt");
        std::fs::remove_dir_all(t.0.join("Projects/big")).unwrap();
        let start = std::time::Instant::now();
        ix.rescan(&[t.0.join("Projects").to_string_lossy().into_owned()], false);
        assert!(start.elapsed() < std::time::Duration::from_millis(500), "{:?}", start.elapsed());
        assert!(ix.search("file1", 5, |_| 0).is_empty());
        assert!(ix.search("sub3", 5, |_| 0).is_empty());
        assert_eq!(names(&ix.search("stay.txt", 5, |_| 0)), ["stay.txt"]);
        assert_eq!(names(&ix.search("after.txt", 5, |_| 0)), ["after.txt"]);
    }

    #[test]
    fn rescan_adds_and_removes() {
        let t = TempTree::new("rescan");
        t.file("Documents/a.txt");
        let (mut ix, _) = Index::crawl(vec![t.root()]);
        t.file("Documents/b.txt");
        t.file("Documents/new/deep/c.txt");
        std::fs::remove_file(t.0.join("Documents/a.txt")).unwrap();
        let docs = t.0.join("Documents").to_string_lossy().into_owned();
        assert!(ix.rescan(&[docs.clone()], false) > 0);
        assert!(ix.search("a.txt", 5, |_| 0).is_empty());
        assert_eq!(names(&ix.search("b.txt", 5, |_| 0)), ["b.txt"]);
        assert_eq!(names(&ix.search("c.txt", 5, |_| 0)), ["c.txt"]);

        std::fs::remove_dir_all(t.0.join("Documents/new")).unwrap();
        ix.rescan(&[docs], false);
        assert!(ix.search("c.txt", 5, |_| 0).is_empty());
        assert!(ix.search("deep", 5, |_| 0).is_empty());

        // An event for a folder the index has not seen rescans its nearest indexed ancestor.
        t.file("Music/x/y/song.mp3");
        ix.rescan(&[t.0.join("Music/x/y").to_string_lossy().into_owned()], false);
        assert_eq!(names(&ix.search("song", 5, |_| 0)), ["song.mp3"]);

        // Events in skipped places are ignored.
        t.file("Library/Caches/zzz.db");
        assert_eq!(ix.rescan(&[t.0.join("Library/Caches").to_string_lossy().into_owned()], false), 0);
    }

    /// `cargo test --release real_home -- --ignored --nocapture`: crawl, memory, snapshot and query
    /// latency on this Mac's real roots.
    #[test]
    #[ignore]
    fn real_home_benchmark() {
        let (ix, stats) = Index::crawl(default_roots());
        eprintln!(
            "crawl: {} entries ({} folders) in {:.0} ms, {:.1} MB in memory, {:.1} bytes/entry",
            stats.entries,
            ix.dir_count(),
            stats.ms,
            ix.bytes() as f64 / 1e6,
            ix.bytes() as f64 / stats.entries as f64
        );
        let file = std::env::temp_dir().join("moo-fsindex-bench.idx");
        let t0 = Instant::now();
        ix.save(&file).unwrap();
        let size = std::fs::metadata(&file).unwrap().len();
        eprintln!("save: {:.0} ms, {:.1} MB on disk", t0.elapsed().as_secs_f64() * 1000.0, size as f64 / 1e6);
        let t0 = Instant::now();
        let back = Index::load(&file, &default_roots()).expect("load");
        eprintln!("load: {:.0} ms", t0.elapsed().as_secs_f64() * 1000.0);
        let _ = std::fs::remove_file(&file);
        for q in ["a", "ma", "main", "readme", "main.tish", "moo main", "pdf", "qrtly", "zzzzqx", "cargo toml"] {
            let t0 = Instant::now();
            let hits = back.search(q, 8, |_| 0);
            let ms = t0.elapsed().as_secs_f64() * 1000.0;
            eprintln!("{q:>12}: {ms:6.2} ms  {:?}", hits.iter().take(3).map(|h| format!("{} ({})", h.name, h.detail)).collect::<Vec<_>>());
        }
    }

    #[test]
    fn snapshot_round_trips_after_compaction() {
        let t = TempTree::new("snap");
        t.file("Documents/keep.txt");
        t.file("Documents/drop.txt");
        let (mut ix, _) = Index::crawl(vec![t.root()]);
        std::fs::remove_file(t.0.join("Documents/drop.txt")).unwrap();
        ix.rescan(&[t.0.join("Documents").to_string_lossy().into_owned()], false);
        ix.event_id = 42;
        ix.compact();
        assert_eq!(ix.dead, 0);
        let file = t.0.join("snapshot.idx");
        ix.save(&file).unwrap();
        let back = Index::load(&file, &[t.root()]).expect("load");
        assert_eq!(back.event_id, 42);
        assert_eq!(back.len(), ix.len());
        assert_eq!(names(&back.search("keep", 5, |_| 0)), ["keep.txt"]);
        assert!(back.search("drop", 5, |_| 0).is_empty());
        let other = Root { path: "/elsewhere".into(), skip_top: vec![], label: "~".into() };
        assert!(Index::load(&file, &[other]).is_none());
    }
}
