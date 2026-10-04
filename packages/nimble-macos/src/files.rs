//! File search over Spotlight's existing index (`MDQuery`, in-process). macOS already pays for
//! that index, so this source adds no crawling and no disk writes of its own.
//!
//! One worker thread serves queries. Only the newest request is kept: keystrokes that arrive while
//! a query runs replace the pending one, and stale results are dropped before delivery.

use std::ffi::c_void;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::Instant;

use dispatch2::DispatchQueue;
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use objc2::rc::Retained;
use objc2_foundation::{NSArray, NSString};

type CFTypeRef = *const c_void;

#[link(name = "CoreServices", kind = "framework")]
extern "C" {
    fn MDQueryCreate(
        alloc: CFTypeRef,
        query: CFTypeRef,
        value_list_attrs: CFTypeRef,
        sorting_attrs: CFTypeRef,
    ) -> CFTypeRef;
    fn MDQuerySetSearchScope(query: CFTypeRef, scope: CFTypeRef, options: u32);
    fn MDQuerySetMaxCount(query: CFTypeRef, size: isize);
    fn MDQueryExecute(query: CFTypeRef, options: usize) -> u8;
    fn MDQueryGetResultCount(query: CFTypeRef) -> isize;
    fn MDQueryGetResultAtIndex(query: CFTypeRef, idx: isize) -> CFTypeRef;
    fn MDItemCopyAttribute(item: CFTypeRef, name: CFTypeRef) -> CFTypeRef;
    static kMDItemPath: CFTypeRef;
    static kMDItemContentType: CFTypeRef;
    static kMDItemLastUsedDate: CFTypeRef;
    static kMDItemFSSize: CFTypeRef;
    static kMDItemFSCreationDate: CFTypeRef;
    static kMDItemFSContentChangeDate: CFTypeRef;
    static kMDQueryScopeHome: CFTypeRef;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(cf: CFTypeRef);
    fn CFDateGetAbsoluteTime(date: CFTypeRef) -> f64;
    fn CFNumberGetValue(number: CFTypeRef, kind: i64, out: *mut c_void) -> bool;
}

const K_CF_NUMBER_SINT64: i64 = 4;
/// CFAbsoluteTime counts from 2001-01-01; Unix time from 1970.
const CF_EPOCH_UNIX: f64 = 978_307_200.0;

const K_MD_QUERY_SYNCHRONOUS: usize = 1;
/// Spotlight returns matches in index order, not by relevance, so fetch a pool and rank it here.
const CANDIDATES: isize = 400;

#[derive(Clone, Debug)]
pub struct FileHit {
    pub name: String,
    pub path: String,
    /// Parent folder, `~`-relative and left-truncated for display.
    pub detail: String,
    pub is_dir: bool,
    pub score: u32,
    /// Seconds since 2001 (CFAbsoluteTime); 0 when the file was never opened.
    pub last_used: f64,
}

const DETAIL_CHARS: usize = 34;

fn detail_for(path: &str, home: &str) -> String {
    let parent = Path::new(path).parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    let short = match parent.strip_prefix(home) {
        Some(rest) if !home.is_empty() => format!("~{rest}"),
        _ => parent,
    };
    let n = short.chars().count();
    if n <= DETAIL_CHARS {
        short
    } else {
        let tail: String = short.chars().skip(n - (DETAIL_CHARS - 1)).collect();
        format!("…{tail}")
    }
}

pub struct Delivery {
    pub generation: u64,
    pub query: String,
    pub hits: Vec<FileHit>,
    pub ms: f64,
}

struct Request {
    generation: u64,
    query: String,
    limit: usize,
}

static LATEST: AtomicU64 = AtomicU64::new(0);
static SLOT: Mutex<Option<Request>> = Mutex::new(None);
static WAKE: Condvar = Condvar::new();
static WORKER: OnceLock<()> = OnceLock::new();

pub fn latest_generation() -> u64 {
    LATEST.load(Ordering::SeqCst)
}

/// Queue a search; `deliver` runs on the main queue with the results unless a newer search started.
pub fn request(query: &str, limit: usize, deliver: fn(Delivery)) -> u64 {
    WORKER.get_or_init(|| {
        std::thread::Builder::new()
            .name("nimble-files".into())
            .spawn(move || worker(deliver))
            .expect("spawn file search worker");
    });
    let generation = LATEST.fetch_add(1, Ordering::SeqCst) + 1;
    *SLOT.lock().unwrap() = Some(Request { generation, query: query.to_string(), limit });
    WAKE.notify_one();
    generation
}

fn worker(deliver: fn(Delivery)) {
    let mut matcher = Matcher::new(Config::DEFAULT.match_paths());
    loop {
        let req = {
            let mut slot = SLOT.lock().unwrap();
            loop {
                if let Some(r) = slot.take() {
                    break r;
                }
                slot = WAKE.wait(slot).unwrap();
            }
        };
        let t0 = Instant::now();
        let hits = run_query(&req.query, req.limit, &mut matcher);
        if req.generation != latest_generation() {
            continue;
        }
        let d = Delivery {
            generation: req.generation,
            query: req.query,
            hits,
            ms: t0.elapsed().as_secs_f64() * 1000.0,
        };
        DispatchQueue::main().exec_async(move || deliver(d));
    }
}

/// Escape a token for an MDQuery string literal, where `*` and `?` are wildcards.
fn escape(token: &str) -> String {
    let mut s = String::with_capacity(token.len());
    for c in token.chars() {
        if matches!(c, '\\' | '"' | '*' | '?' | '\'') {
            s.push('\\');
        }
        s.push(c);
    }
    s
}

fn spotlight_query(query: &str) -> Option<String> {
    let clauses: Vec<String> = query
        .split_whitespace()
        .map(|t| format!("(kMDItemFSName == \"*{}*\"cd)", escape(t)))
        .collect();
    if clauses.is_empty() {
        return None;
    }
    // Apps come from the app index.
    Some(format!(
        "{} && (kMDItemContentType != \"com.apple.application-bundle\")",
        clauses.join(" && ")
    ))
}

/// Skip places a launcher should not surface: dot-dirs, dependency trees, bundle internals, ~/Library.
fn wanted(path: &str, home: &str) -> bool {
    if !home.is_empty() && path.starts_with(&format!("{home}/Library/")) {
        return false;
    }
    if path.contains(".app/") {
        return false;
    }
    !Path::new(path).components().any(|c| {
        let s = c.as_os_str().to_string_lossy();
        s.starts_with('.') || s == "node_modules" || s == "target"
    })
}

unsafe fn copy_string(item: CFTypeRef, attr: CFTypeRef) -> Option<String> {
    let v = MDItemCopyAttribute(item, attr);
    if v.is_null() {
        return None;
    }
    // MDItemCopyAttribute returns +1; CFString is toll-free bridged to NSString.
    let s: Retained<NSString> = Retained::from_raw(v as *mut NSString)?;
    Some(s.to_string())
}

unsafe fn copy_date(item: CFTypeRef, attr: CFTypeRef) -> f64 {
    let v = MDItemCopyAttribute(item, attr);
    if v.is_null() {
        return 0.0;
    }
    let t = CFDateGetAbsoluteTime(v);
    CFRelease(v);
    t
}

fn run_query(query: &str, limit: usize, matcher: &mut Matcher) -> Vec<FileHit> {
    let Some(q) = spotlight_query(query) else { return Vec::new() };
    let home = std::env::var("HOME").unwrap_or_default();
    let mut found = Vec::new();
    unsafe {
        let qs = NSString::from_str(&q);
        let mdq = MDQueryCreate(
            std::ptr::null(),
            Retained::as_ptr(&qs) as CFTypeRef,
            std::ptr::null(),
            std::ptr::null(),
        );
        if mdq.is_null() {
            return Vec::new();
        }
        let scope: Retained<NSArray<NSString>> =
            NSArray::from_slice(&[&*(kMDQueryScopeHome as *const NSString)]);
        MDQuerySetSearchScope(mdq, Retained::as_ptr(&scope) as CFTypeRef, 0);
        MDQuerySetMaxCount(mdq, CANDIDATES);
        if MDQueryExecute(mdq, K_MD_QUERY_SYNCHRONOUS) != 0 {
            for i in 0..MDQueryGetResultCount(mdq) {
                let item = MDQueryGetResultAtIndex(mdq, i);
                let Some(path) = copy_string(item, kMDItemPath) else { continue };
                if !wanted(&path, &home) {
                    continue;
                }
                let is_dir = copy_string(item, kMDItemContentType).as_deref() == Some("public.folder");
                let last_used = copy_date(item, kMDItemLastUsedDate);
                found.push((path, is_dir, last_used));
            }
        }
        CFRelease(mdq);
    }

    let pattern = Pattern::parse(query.trim(), CaseMatching::Ignore, Normalization::Smart);
    let mut buf = Vec::new();
    let mut hits: Vec<FileHit> = found
        .into_iter()
        .filter_map(|(path, is_dir, last_used)| {
            let name = Path::new(&path).file_name()?.to_string_lossy().into_owned();
            let score = pattern.score(Utf32Str::new(&name, &mut buf), matcher)?;
            let detail = detail_for(&path, &home);
            Some(FileHit { name, path, detail, is_dir, score, last_used })
        })
        .collect();
    hits.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then(b.last_used.total_cmp(&a.last_used))
            .then(a.path.len().cmp(&b.path.len()))
    });
    hits.truncate(limit);
    hits
}

// ── Metadata search: size, dates, kind and folder as well as name (synchronous) ──

#[derive(Clone, Debug, Default)]
pub struct Filter {
    /// Words that must all appear in the file name.
    pub name: String,
    /// `image`, `video`, `audio`, `pdf`, `document`, `text`, `spreadsheet`, `presentation`,
    /// `archive`, `diskimage`, `folder`; anything else means any kind.
    pub kind: String,
    pub min_bytes: u64,
    /// 0: no upper limit.
    pub max_bytes: u64,
    /// Created, modified or last opened within this many seconds; 0: any time.
    pub created_secs: f64,
    pub modified_secs: f64,
    pub opened_secs: f64,
    /// Absolute folder to search; empty: the home folder.
    pub folder: String,
    /// `size` (largest first), `created`, `modified`, `opened` (newest first) or `name`.
    pub sort: String,
    pub limit: usize,
}

#[derive(Clone, Debug)]
pub struct Found {
    pub name: String,
    pub path: String,
    pub detail: String,
    pub is_dir: bool,
    pub bytes: u64,
    /// Unix seconds; 0 when unknown (opened: never opened).
    pub created: f64,
    pub modified: f64,
    pub opened: f64,
}

pub struct FindResult {
    pub hits: Vec<Found>,
    /// Matches before `limit` (capped by FIND_CANDIDATES).
    pub total: usize,
    pub ms: f64,
}

const FIND_CANDIDATES: isize = 2000;
/// Without a size floor Spotlight would hand back an arbitrary 2000 files, so "largest" narrows
/// from the top until enough files qualify.
const SIZE_STEPS: [u64; 5] = [1 << 30, 100 << 20, 10 << 20, 1 << 20, 0];
/// The same for "newest" without a date range: the last day, week, month, year, then any time.
const AGE_STEPS: [f64; 5] = [86_400.0, 604_800.0, 2_592_000.0, 31_536_000.0, 0.0];

fn kind_clause(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "image" => "kMDItemContentTypeTree == \"public.image\"",
        "video" => "kMDItemContentTypeTree == \"public.movie\"",
        "audio" => "kMDItemContentTypeTree == \"public.audio\"",
        "pdf" => "kMDItemContentTypeTree == \"com.adobe.pdf\"",
        "document" => "(kMDItemContentTypeTree == \"public.composite-content\" || kMDItemContentTypeTree == \"public.text\")",
        "text" => "kMDItemContentTypeTree == \"public.text\"",
        "spreadsheet" => "kMDItemContentTypeTree == \"public.spreadsheet\"",
        "presentation" => "kMDItemContentTypeTree == \"public.presentation\"",
        "archive" => "kMDItemContentTypeTree == \"public.archive\"",
        "diskimage" => "kMDItemContentTypeTree == \"public.disk-image\"",
        "folder" => "kMDItemContentType == \"public.folder\"",
        _ => return None,
    })
}

fn date_attr(sort: &str) -> &'static str {
    match sort {
        "created" => "kMDItemFSCreationDate",
        "opened" => "kMDItemLastUsedDate",
        _ => "kMDItemFSContentChangeDate",
    }
}

fn filter_query(f: &Filter, min_bytes: u64, age: Option<(&str, f64)>) -> String {
    let mut c: Vec<String> = f
        .name
        .split_whitespace()
        .map(|t| format!("(kMDItemFSName == \"*{}*\"cd)", escape(t)))
        .collect();
    if let Some(k) = kind_clause(&f.kind) {
        c.push(format!("({k})"));
    }
    if min_bytes > 0 {
        c.push(format!("(kMDItemFSSize >= {min_bytes})"));
    }
    if f.max_bytes > 0 {
        c.push(format!("(kMDItemFSSize <= {})", f.max_bytes));
    }
    let dates = [
        ("kMDItemFSCreationDate", f.created_secs),
        ("kMDItemFSContentChangeDate", f.modified_secs),
        ("kMDItemLastUsedDate", f.opened_secs),
    ];
    for (attr, secs) in dates.into_iter().chain(age) {
        if secs > 0.0 {
            c.push(format!("({attr} >= $time.now(-{}))", secs.round() as i64));
        }
    }
    c.push("(kMDItemContentType != \"com.apple.application-bundle\")".into());
    c.join(" && ")
}

/// Questions like "what is taking up space" need build output, caches and hidden folders too, so
/// unlike the launcher's name search only the insides of app bundles are skipped.
fn wanted_in(path: &str, folder: &str) -> bool {
    !path.strip_prefix(folder).unwrap_or(path).contains(".app/")
}

unsafe fn copy_u64(item: CFTypeRef, attr: CFTypeRef) -> u64 {
    let v = MDItemCopyAttribute(item, attr);
    if v.is_null() {
        return 0;
    }
    let mut n: i64 = 0;
    CFNumberGetValue(v, K_CF_NUMBER_SINT64, &mut n as *mut i64 as *mut c_void);
    CFRelease(v);
    n.max(0) as u64
}

fn unix(cf: f64) -> f64 {
    if cf == 0.0 { 0.0 } else { cf + CF_EPOCH_UNIX }
}

fn md_find(query: &str, folder: &str, home: &str) -> Vec<Found> {
    let mut out = Vec::new();
    unsafe {
        let qs = NSString::from_str(query);
        let mdq = MDQueryCreate(std::ptr::null(), Retained::as_ptr(&qs) as CFTypeRef, std::ptr::null(), std::ptr::null());
        if mdq.is_null() {
            return out;
        }
        let path = NSString::from_str(folder);
        let scope: Retained<NSArray<NSString>> = if folder.is_empty() {
            NSArray::from_slice(&[&*(kMDQueryScopeHome as *const NSString)])
        } else {
            NSArray::from_slice(&[&*path])
        };
        MDQuerySetSearchScope(mdq, Retained::as_ptr(&scope) as CFTypeRef, 0);
        MDQuerySetMaxCount(mdq, FIND_CANDIDATES);
        if MDQueryExecute(mdq, K_MD_QUERY_SYNCHRONOUS) != 0 {
            for i in 0..MDQueryGetResultCount(mdq) {
                let item = MDQueryGetResultAtIndex(mdq, i);
                let Some(path) = copy_string(item, kMDItemPath) else { continue };
                if !wanted_in(&path, folder) {
                    continue;
                }
                let Some(name) = Path::new(&path).file_name().map(|n| n.to_string_lossy().into_owned()) else { continue };
                out.push(Found {
                    name,
                    detail: detail_for(&path, home),
                    is_dir: copy_string(item, kMDItemContentType).as_deref() == Some("public.folder"),
                    bytes: copy_u64(item, kMDItemFSSize),
                    created: unix(copy_date(item, kMDItemFSCreationDate)),
                    modified: unix(copy_date(item, kMDItemFSContentChangeDate)),
                    opened: unix(copy_date(item, kMDItemLastUsedDate)),
                    path,
                });
            }
        }
        CFRelease(mdq);
    }
    out
}

/// Spotlight query for `f`, sorted and cut to `f.limit`. Sorting by size or date with no range of
/// that kind narrows from the largest / newest so the top of the list is right.
pub fn find(f: &Filter) -> FindResult {
    let t0 = Instant::now();
    let home = std::env::var("HOME").unwrap_or_default();
    let folder = f.folder.trim_end_matches('/').to_string();
    let date_sort = matches!(f.sort.as_str(), "created" | "modified" | "opened");
    let has_range = match f.sort.as_str() {
        "created" => f.created_secs > 0.0,
        "opened" => f.opened_secs > 0.0,
        _ => f.modified_secs > 0.0,
    };
    let mut hits = Vec::new();
    if f.sort == "size" && f.min_bytes == 0 {
        for step in SIZE_STEPS {
            hits = md_find(&filter_query(f, step, None), &folder, &home);
            if hits.len() >= f.limit {
                break;
            }
        }
    } else if date_sort && !has_range {
        for step in AGE_STEPS {
            hits = md_find(&filter_query(f, f.min_bytes, Some((date_attr(&f.sort), step))), &folder, &home);
            if hits.len() >= f.limit {
                break;
            }
        }
    } else {
        hits = md_find(&filter_query(f, f.min_bytes, None), &folder, &home);
    }
    match f.sort.as_str() {
        "size" => hits.sort_by(|a, b| b.bytes.cmp(&a.bytes)),
        "created" => hits.sort_by(|a, b| b.created.total_cmp(&a.created)),
        "opened" => hits.sort_by(|a, b| b.opened.total_cmp(&a.opened)),
        "name" => hits.sort_by_key(|h| h.name.to_lowercase()),
        _ => hits.sort_by(|a, b| b.modified.total_cmp(&a.modified)),
    }
    let total = hits.len();
    hits.truncate(f.limit);
    FindResult { hits, total, ms: t0.elapsed().as_secs_f64() * 1000.0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_query_combines_clauses() {
        let f = Filter { name: "tax 2025".into(), kind: "pdf".into(), created_secs: 172_800.0, ..Default::default() };
        let q = filter_query(&f, 100 << 20, None);
        assert_eq!(
            q,
            "(kMDItemFSName == \"*tax*\"cd) && (kMDItemFSName == \"*2025*\"cd) && (kMDItemContentTypeTree == \"com.adobe.pdf\") \
             && (kMDItemFSSize >= 104857600) && (kMDItemFSCreationDate >= $time.now(-172800)) \
             && (kMDItemContentType != \"com.apple.application-bundle\")"
        );
    }

    /// Real Spotlight: everything returned honours the filter and comes back largest first.
    #[test]
    #[ignore = "queries this Mac's Spotlight index"]
    fn find_large_recent_files() {
        let f = Filter { sort: "size".into(), modified_secs: 30.0 * 86_400.0, limit: 5, ..Default::default() };
        let r = find(&f);
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64();
        for w in r.hits.windows(2) {
            assert!(w[0].bytes >= w[1].bytes);
        }
        for h in &r.hits {
            assert!(now - h.modified <= 30.0 * 86_400.0 + 60.0, "{h:?}");
            println!("{:>12} {} {}", h.bytes, h.path, r.ms);
        }
    }
}
