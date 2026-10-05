//! Saved AI chats, one JSON file each in `~/Library/Application Support/Moo/chats`
//! (`MOO_CHATS` for the folder). The shell owns the format; this only needs `title`, `model`
//! and `updated` (Unix milliseconds) at the top level to list them.

use std::path::PathBuf;

use tishlang_core::{json_parse, Value};

pub fn dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("MOO_CHATS") {
        return Some(PathBuf::from(d));
    }
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support/Moo/chats"))
}

fn valid(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn now_ms() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)
}

/// Save `json` as chat `id`, or as a new chat when `id` is empty. Returns the id.
pub fn save(id: &str, json: &str) -> Result<String, String> {
    let id = if id.is_empty() { now_ms().to_string() } else { id.to_string() };
    if !valid(&id) {
        return Err(format!("bad chat id `{id}`"));
    }
    let dir = dir().ok_or("HOME is not set")?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let path = dir.join(format!("{id}.json"));
    let tmp = dir.join(format!("{id}.json.tmp"));
    std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
    Ok(id)
}

pub fn load(id: &str) -> Option<String> {
    if !valid(id) {
        return None;
    }
    std::fs::read_to_string(dir()?.join(format!("{id}.json"))).ok()
}

pub fn delete(id: &str) -> bool {
    valid(id) && dir().is_some_and(|d| std::fs::remove_file(d.join(format!("{id}.json"))).is_ok())
}

pub struct Meta {
    pub id: String,
    pub title: String,
    pub model: String,
    pub updated: f64,
}

fn text(v: &Value, key: &str) -> String {
    match v {
        Value::Object(o) => match o.borrow().strings.get(key) {
            Some(Value::String(s)) => s.to_string(),
            Some(Value::Number(n)) => n.to_string(),
            _ => String::new(),
        },
        _ => String::new(),
    }
}

/// Every chat, most recently updated first.
pub fn list() -> Vec<Meta> {
    let Some(entries) = dir().and_then(|d| std::fs::read_dir(d).ok()) else { return Vec::new() };
    let mut out: Vec<Meta> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let id = name.strip_suffix(".json")?.to_string();
            let v = json_parse(&std::fs::read_to_string(e.path()).ok()?).ok()?;
            Some(Meta { title: text(&v, "title"), model: text(&v, "model"), updated: text(&v, "updated").parse().unwrap_or(0.0), id })
        })
        .collect();
    out.sort_by(|a, b| b.updated.partial_cmp(&a.updated).unwrap_or(std::cmp::Ordering::Equal));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_list_load_delete() {
        let d = std::env::temp_dir().join(format!("moo-chats-{}", std::process::id()));
        std::env::set_var("MOO_CHATS", &d);
        let a = save("", r#"{"title":"First","model":"apple","updated":1}"#).unwrap();
        save("b-2", r#"{"title":"Second","model":"ollama:x","updated":2}"#).unwrap();
        let l = list();
        assert_eq!(l.iter().map(|m| m.title.as_str()).collect::<Vec<_>>(), ["Second", "First"]);
        assert_eq!(l[0].model, "ollama:x");
        assert!(load(&a).unwrap().contains("First"));
        assert!(save("../x", "{}").is_err());
        assert_eq!(load("../x"), None);
        assert!(delete("b-2"));
        assert_eq!(list().len(), 1);
        let _ = std::fs::remove_dir_all(d);
    }
}
