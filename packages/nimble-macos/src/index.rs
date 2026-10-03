//! Application index and fuzzy ranking. Portable: no AppKit.

use std::cell::RefCell;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

#[derive(Clone, Debug)]
pub struct AppEntry {
    pub name: String,
    pub path: String,
}

thread_local! {
    static APPS: RefCell<Vec<AppEntry>> = const { RefCell::new(Vec::new()) };
    static MATCHER: RefCell<Matcher> = RefCell::new(Matcher::new(Config::DEFAULT));
}

fn roots() -> Vec<PathBuf> {
    let mut r: Vec<PathBuf> = [
        "/Applications",
        "/Applications/Utilities",
        "/System/Applications",
        "/System/Applications/Utilities",
        "/System/Library/CoreServices/Applications",
    ]
    .iter()
    .map(PathBuf::from)
    .collect();
    if let Some(home) = std::env::var_os("HOME") {
        r.push(Path::new(&home).join("Applications"));
    }
    r
}

pub fn root_strings() -> Vec<String> {
    roots().iter().map(|p| p.to_string_lossy().into_owned()).collect()
}

pub fn app_count() -> usize {
    APPS.with(|a| a.borrow().len())
}

fn is_app(p: &Path) -> bool {
    p.extension().is_some_and(|e| e == "app")
}

/// `depth` lets folders such as `/Applications/Setapp/` contribute their bundles.
fn scan_dir(dir: &Path, depth: u32, seen: &mut HashSet<String>, out: &mut Vec<AppEntry>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        let p = entry.path();
        if is_app(&p) {
            let Some(stem) = p.file_stem().and_then(|s| s.to_str()) else { continue };
            if seen.insert(stem.to_lowercase()) {
                out.push(AppEntry { name: stem.to_string(), path: p.to_string_lossy().into_owned() });
            }
        } else if depth > 0 && entry.file_type().is_ok_and(|t| t.is_dir()) {
            scan_dir(&p, depth - 1, seen, out);
        }
    }
}

/// Rebuild the index. Returns (count, elapsed milliseconds).
pub fn reindex() -> (usize, f64) {
    let t0 = Instant::now();
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for root in roots() {
        scan_dir(&root, 1, &mut seen, &mut out);
    }
    let finder = Path::new("/System/Library/CoreServices/Finder.app");
    if finder.exists() && seen.insert("finder".into()) {
        out.push(AppEntry { name: "Finder".into(), path: finder.to_string_lossy().into_owned() });
    }
    out.sort_by_key(|a| a.name.to_lowercase());
    let n = out.len();
    APPS.with(|a| *a.borrow_mut() = out);
    (n, t0.elapsed().as_secs_f64() * 1000.0)
}

/// Rank arbitrary titles (plugin commands, plugin list items) with the same matcher as apps.
/// Returns `(index, score)` best first; an empty query keeps the original order.
pub fn fuzzy(query: &str, titles: &[String], limit: usize) -> Vec<(usize, u32)> {
    let q = query.trim();
    if q.is_empty() {
        return (0..titles.len().min(limit)).map(|i| (i, 0)).collect();
    }
    let pattern = Pattern::parse(q, CaseMatching::Ignore, Normalization::Smart);
    let mut buf = Vec::new();
    let mut scored: Vec<(usize, u32)> = MATCHER.with(|m| {
        let mut m = m.borrow_mut();
        titles
            .iter()
            .enumerate()
            .filter_map(|(i, t)| pattern.score(Utf32Str::new(t, &mut buf), &mut m).map(|s| (i, s)))
            .collect()
    });
    scored.sort_by(|x, y| y.1.cmp(&x.1).then(titles[x.0].len().cmp(&titles[y.0].len())));
    scored.truncate(limit);
    scored
}

/// Plugins directly inside `dir`, sorted: native modules (`*.lib`, `*.dylib`) and bytecode (`*.tishc`).
pub fn plugin_paths(dir: &str) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<String> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "lib" || e == "dylib" || e == "tishc"))
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    out.sort();
    out
}

/// Best `limit` matches for `query`; an empty query lists apps alphabetically.
pub fn search(query: &str, limit: usize) -> Vec<(AppEntry, u32)> {
    APPS.with(|apps| {
        let apps = apps.borrow();
        let q = query.trim();
        if q.is_empty() {
            return apps.iter().take(limit).map(|a| (a.clone(), 0)).collect();
        }
        let pattern = Pattern::parse(q, CaseMatching::Ignore, Normalization::Smart);
        let mut buf = Vec::new();
        let mut scored: Vec<(&AppEntry, u32)> = MATCHER.with(|m| {
            let mut m = m.borrow_mut();
            apps.iter()
                .filter_map(|a| pattern.score(Utf32Str::new(&a.name, &mut buf), &mut m).map(|s| (a, s)))
                .collect()
        });
        scored.sort_by(|x, y| y.1.cmp(&x.1).then(x.0.name.len().cmp(&y.0.name.len())));
        scored.into_iter().take(limit).map(|(a, s)| (a.clone(), s)).collect()
    })
}
