//! Search history: queries the user opened something from, newest first, as Spotlight shows on ↑.
//! One query per line; a repeated query moves to the top. Portable: no AppKit.

use std::cell::RefCell;
use std::path::PathBuf;

const MAX_ENTRIES: usize = 50;

thread_local! {
    static STORE: RefCell<Option<Vec<String>>> = const { RefCell::new(None) };
}

pub fn store_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("MOO_HISTORY") {
        return Some(PathBuf::from(p));
    }
    let home = PathBuf::from(std::env::var_os("HOME")?);
    #[cfg(target_os = "macos")]
    let dir = home.join("Library/Application Support/Moo");
    #[cfg(not(target_os = "macos"))]
    let dir = home.join(".local/share/moo");
    Some(dir.join("history.txt"))
}

fn load() -> Vec<String> {
    let Some(text) = store_path().and_then(|p| std::fs::read_to_string(p).ok()) else { return Vec::new() };
    text.lines().filter(|l| !l.trim().is_empty()).take(MAX_ENTRIES).map(str::to_string).collect()
}

fn with_store<R>(f: impl FnOnce(&mut Vec<String>) -> R) -> R {
    STORE.with(|s| f(s.borrow_mut().get_or_insert_with(load)))
}

fn save(items: &[String]) -> std::io::Result<()> {
    let Some(path) = store_path() else { return Ok(()) };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("txt.tmp");
    std::fs::write(&tmp, items.join("\n") + "\n")?;
    std::fs::rename(tmp, path)
}

/// Put `query` at the top of the history and persist.
pub fn add(query: &str) {
    let q = query.trim().replace('\n', " ");
    if q.is_empty() {
        return;
    }
    with_store(|items| {
        items.retain(|i| *i != q);
        items.insert(0, q);
        items.truncate(MAX_ENTRIES);
        if let Err(e) = save(items) {
            eprintln!("moo: history save failed: {e}");
        }
    });
}

/// Up to `limit` queries, newest first.
pub fn list(limit: usize) -> Vec<String> {
    with_store(|items| items.iter().take(limit).cloned().collect())
}

pub fn clear() {
    with_store(|items| {
        items.clear();
        if let Err(e) = save(items) {
            eprintln!("moo: history save failed: {e}");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newest_first_without_duplicates() {
        let path = std::env::temp_dir().join(format!("moo-history-{}.txt", std::process::id()));
        std::env::set_var("MOO_HISTORY", &path);
        let _ = std::fs::remove_file(&path);
        add("safari");
        add("  notes ");
        add("safari");
        assert_eq!(list(10), vec!["safari", "notes"]);
        STORE.with(|s| *s.borrow_mut() = None);
        assert_eq!(list(1), vec!["safari"], "reloaded from disk");
        clear();
        assert!(list(10).is_empty());
        let _ = std::fs::remove_file(&path);
    }
}
