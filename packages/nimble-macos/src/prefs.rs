//! View preferences the launcher remembers between runs (the Applications grid or list, ...).
//! One `key<TAB>value` per line. Portable: no AppKit.

use std::cell::RefCell;
use std::path::PathBuf;

thread_local! {
    static STORE: RefCell<Option<Vec<(String, String)>>> = const { RefCell::new(None) };
}

pub fn store_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("NIMBLE_PREFS") {
        return Some(PathBuf::from(p));
    }
    let home = PathBuf::from(std::env::var_os("HOME")?);
    #[cfg(target_os = "macos")]
    let dir = home.join("Library/Application Support/Nimble");
    #[cfg(not(target_os = "macos"))]
    let dir = home.join(".local/share/nimble");
    Some(dir.join("prefs.tsv"))
}

fn load() -> Vec<(String, String)> {
    let Some(text) = store_path().and_then(|p| std::fs::read_to_string(p).ok()) else { return Vec::new() };
    text.lines().filter_map(|l| l.split_once('\t')).map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

fn with_store<R>(f: impl FnOnce(&mut Vec<(String, String)>) -> R) -> R {
    STORE.with(|s| f(s.borrow_mut().get_or_insert_with(load)))
}

fn save(items: &[(String, String)]) -> std::io::Result<()> {
    let Some(path) = store_path() else { return Ok(()) };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text: String = items.iter().map(|(k, v)| format!("{k}\t{v}\n")).collect();
    let tmp = path.with_extension("tsv.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(tmp, path)
}

/// The stored value, or "" when unset.
pub fn get(key: &str) -> String {
    with_store(|items| items.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone()).unwrap_or_default())
}

pub fn set(key: &str, value: &str) {
    let clean = |s: &str| s.replace(['\t', '\n'], " ");
    let (key, value) = (clean(key.trim()), clean(value));
    if key.is_empty() {
        return;
    }
    with_store(|items| {
        match items.iter_mut().find(|(k, _)| *k == key) {
            Some(item) => item.1 = value,
            None => items.push((key, value)),
        }
        if let Err(e) = save(items) {
            eprintln!("nimble: prefs save failed: {e}");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_get_and_reload() {
        let path = std::env::temp_dir().join(format!("nimble-prefs-{}.tsv", std::process::id()));
        std::env::set_var("NIMBLE_PREFS", &path);
        let _ = std::fs::remove_file(&path);
        assert_eq!(get("appsView"), "");
        set("appsView", "list");
        set("other", "a\tb");
        set("appsView", "grid");
        STORE.with(|s| *s.borrow_mut() = None);
        assert_eq!(get("appsView"), "grid", "reloaded from disk");
        assert_eq!(get("other"), "a b");
        let _ = std::fs::remove_file(&path);
    }
}
