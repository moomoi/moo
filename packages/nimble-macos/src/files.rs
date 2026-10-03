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
    static kMDQueryScopeHome: CFTypeRef;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(cf: CFTypeRef);
    fn CFDateGetAbsoluteTime(date: CFTypeRef) -> f64;
}

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
