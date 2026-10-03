//! Frecency: how often and how recently each item (app path, `plugin:id/cmd`, ...) was opened.
//! A use adds 1 to a score that halves every `HALF_LIFE_DAYS`. Stored as a small TSV that is
//! written only when a use is recorded, never on reads or searches. Portable: no AppKit.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const HALF_LIFE_DAYS: f64 = 7.0;
const MAX_ENTRIES: usize = 500;
const MIN_SCORE: f64 = 0.05;
/// Points added to a match score per unit of `ln(1 + frecency)`. One recent use adds ~14,
/// ten add ~48: enough to reorder close matches, not to beat a much better match.
const BOOST_SCALE: f64 = 20.0;

#[derive(Clone, Copy)]
struct Entry {
    score: f64,
    at: f64,
}

thread_local! {
    static STORE: RefCell<Option<HashMap<String, Entry>>> = const { RefCell::new(None) };
}

fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

fn decayed(e: Entry, t: f64) -> f64 {
    let days = ((t - e.at) / 86_400.0).max(0.0);
    e.score * 0.5f64.powf(days / HALF_LIFE_DAYS)
}

pub fn store_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("NIMBLE_FRECENCY") {
        return Some(PathBuf::from(p));
    }
    let home = PathBuf::from(std::env::var_os("HOME")?);
    #[cfg(target_os = "macos")]
    let dir = home.join("Library/Application Support/Nimble");
    #[cfg(not(target_os = "macos"))]
    let dir = home.join(".local/share/nimble");
    Some(dir.join("frecency.tsv"))
}

fn load() -> HashMap<String, Entry> {
    let mut m = HashMap::new();
    let Some(text) = store_path().and_then(|p| std::fs::read_to_string(p).ok()) else { return m };
    for line in text.lines() {
        let mut parts = line.splitn(3, '\t');
        let (Some(s), Some(a), Some(k)) = (parts.next(), parts.next(), parts.next()) else { continue };
        if let (Ok(score), Ok(at)) = (s.parse(), a.parse()) {
            m.insert(k.to_string(), Entry { score, at });
        }
    }
    m
}

fn with_store<R>(f: impl FnOnce(&mut HashMap<String, Entry>) -> R) -> R {
    STORE.with(|s| f(s.borrow_mut().get_or_insert_with(load)))
}

fn save(m: &HashMap<String, Entry>) -> std::io::Result<()> {
    let Some(path) = store_path() else { return Ok(()) };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut text = String::new();
    for (k, e) in m {
        if !k.contains('\n') {
            text.push_str(&format!("{:.4}\t{:.0}\t{}\n", e.score, e.at, k));
        }
    }
    let tmp = path.with_extension("tsv.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(tmp, path)
}

/// Record one use of `key` and persist. Prunes faded entries so the file stays small.
pub fn record(key: &str) {
    if key.is_empty() {
        return;
    }
    let t = now();
    with_store(|m| {
        let score = m.get(key).map_or(0.0, |e| decayed(*e, t)) + 1.0;
        m.insert(key.to_string(), Entry { score, at: t });
        m.retain(|_, e| decayed(*e, t) >= MIN_SCORE);
        if m.len() > MAX_ENTRIES {
            let mut scores: Vec<f64> = m.values().map(|e| decayed(*e, t)).collect();
            scores.sort_by(|a, b| b.total_cmp(a));
            let cut = scores[MAX_ENTRIES - 1];
            m.retain(|_, e| decayed(*e, t) >= cut);
        }
        if let Err(e) = save(m) {
            eprintln!("nimble: frecency save failed: {e}");
        }
    });
}

/// Current decayed score for `key` (0 when never used).
pub fn score(key: &str) -> f64 {
    let t = now();
    with_store(|m| m.get(key).map_or(0.0, |e| decayed(*e, t)))
}

/// Score bonus to add to a fuzzy match score.
pub fn boost(key: &str) -> u32 {
    (BOOST_SCALE * score(key).ln_1p()).round() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decay_halves_per_half_life() {
        let e = Entry { score: 4.0, at: 0.0 };
        let t = HALF_LIFE_DAYS * 86_400.0;
        assert!((decayed(e, t) - 2.0).abs() < 1e-9);
        assert!((decayed(e, 2.0 * t) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn record_persists_and_boosts() {
        let path = std::env::temp_dir().join(format!("nimble-frecency-{}.tsv", std::process::id()));
        std::env::set_var("NIMBLE_FRECENCY", &path);
        let _ = std::fs::remove_file(&path);
        assert_eq!(boost("/Applications/Safari.app"), 0);
        record("/Applications/Safari.app");
        record("/Applications/Safari.app");
        assert!(boost("/Applications/Safari.app") > boost("/Applications/Mail.app"));
        STORE.with(|s| *s.borrow_mut() = None);
        assert!((score("/Applications/Safari.app") - 2.0).abs() < 0.01, "reloaded from disk");
        let _ = std::fs::remove_file(&path);
    }
}
