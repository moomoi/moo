//! User shortcuts and hotkeys, stored as hand-editable JSON in `~/.config/moo/shortcuts.json`
//! (`$XDG_CONFIG_HOME/moo`, or `MOO_CONFIG` for the file itself):
//!
//! ```json
//! {
//!   "launcher": "cmd+space",
//!   "shortcuts": [
//!     { "keyword": "g", "name": "Google", "kind": "url", "target": "https://www.google.com/search?q={query}" },
//!     { "keyword": "dl", "name": "Downloads", "kind": "open", "target": "~/Downloads", "hotkey": "ctrl+alt+d" }
//!   ],
//!   "hotkeys": [{ "keys": "cmd+shift+v", "run": "moo:clipboard" }],
//!   "search": "https://duckduckgo.com/?q={query}",
//!   "ai": { "model": "hypery:gpt-5-mini", "providers": { "work": { "url": "https://llm.example/v1", "keyEnv": "WORK_KEY" } } }
//! }
//! ```
//!
//! Type a keyword, a space and some text: the text fills `{query}`. Kinds: `url` opens the
//! expanded URL (query percent-encoded), `open` opens an app, file or folder, `command` runs a
//! Moo or plugin command with `input` as its search text, `shell` runs `/bin/sh -c` (query
//! single-quoted) and shows, copies or discards the output, `text` copies the expanded text (or,
//! with `"expand": true`, replaces the keyword wherever it is typed), `ai` sends the expanded
//! prompt to `model` (or the default model). Templates also take `{clipboard}`, `{selection}` (the
//! text selected in the frontmost app), `{date}` (2026-10-03) and `{time}` (17:20).

use std::path::{Path, PathBuf};

use tishlang_core::{json_parse, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Url,
    Open,
    Command,
    Shell,
    Text,
    Ai,
}

pub const KINDS: [Kind; 6] = [Kind::Url, Kind::Open, Kind::Command, Kind::Shell, Kind::Text, Kind::Ai];

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Url => "url",
            Kind::Open => "open",
            Kind::Command => "command",
            Kind::Shell => "shell",
            Kind::Text => "text",
            Kind::Ai => "ai",
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        KINDS.into_iter().find(|k| k.name() == s)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shortcut {
    pub keyword: String,
    pub name: String,
    pub kind: Kind,
    pub target: String,
    /// `command` only: the search text to start the command with. Default `{query}`.
    pub input: String,
    /// `shell` only: `show` (default), `copy` or `none`.
    pub output: String,
    /// Optional global hotkey that runs the shortcut without opening the launcher.
    pub hotkey: String,
    /// `ai` only: `provider:model`; empty uses the default model.
    pub model: String,
    /// `text` only: expand the keyword as a snippet wherever it is typed.
    pub expand: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HotkeyBinding {
    pub keys: String,
    /// What to run: `moo:toggle`, `moo:clipboard`, `plugin:<id>/<command>`, a shortcut
    /// keyword, ...
    pub run: String,
    /// Text to run it with (fills `{query}` / the command's search field).
    pub query: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Config {
    /// Launcher hotkey; empty means Moo's default.
    pub launcher: String,
    pub shortcuts: Vec<Shortcut>,
    pub hotkeys: Vec<HotkeyBinding>,
    /// Web search URL with `{query}`; empty means Google.
    pub search: String,
    pub ai: AiConfig,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AiConfig {
    /// Default model, `provider:model` or `apple`; empty means Apple's on-device model.
    pub model: String,
    /// Added providers, or overrides of built-in ones, by id.
    pub providers: Vec<ProviderConfig>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProviderConfig {
    pub id: String,
    pub title: String,
    /// OpenAI-compatible base URL (ends before `/chat/completions`).
    pub url: String,
    /// Environment variable holding the API key.
    pub key_env: String,
    /// OAuth client id, for providers with browser sign-in.
    pub client_id: String,
    /// A web page registered as the OAuth redirect that forwards the code to Moo's loopback
    /// listener; empty to register `http://127.0.0.1/callback` directly.
    pub redirect_uri: String,
    /// The organization (team id) that requests act for and bill to; empty for the personal one.
    pub organization: String,
}

pub const OUTPUTS: [&str; 3] = ["show", "copy", "none"];
const MAX_KEYWORD: usize = 32;

pub fn config_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("MOO_CONFIG") {
        return Some(PathBuf::from(p));
    }
    if let Some(x) = std::env::var_os("XDG_CONFIG_HOME").filter(|x| !x.is_empty()) {
        return Some(PathBuf::from(x).join("moo/shortcuts.json"));
    }
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config/moo/shortcuts.json"))
}

fn field(v: &Value, key: &str) -> Option<Value> {
    match v {
        Value::Object(o) => o.borrow().strings.get(key).cloned(),
        _ => None,
    }
}

fn text(v: &Value, key: &str) -> String {
    match field(v, key) {
        Some(Value::String(s)) => s.to_string(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

fn items(v: &Value, key: &str) -> Vec<Value> {
    match field(v, key) {
        Some(Value::Array(a)) => a.borrow().clone(),
        _ => Vec::new(),
    }
}

/// Parse the file's text. Invalid entries are skipped with a warning each, so one typo does not
/// lose every shortcut; malformed JSON is an error.
pub fn parse(json: &str) -> Result<(Config, Vec<String>), String> {
    if json.trim().is_empty() {
        return Ok((Config::default(), Vec::new()));
    }
    let root = json_parse(json)?;
    if !matches!(root, Value::Object(_)) {
        return Err("the file must contain a JSON object".into());
    }
    let mut cfg = Config { launcher: text(&root, "launcher"), search: text(&root, "search"), ..Config::default() };
    if let Some(ai) = field(&root, "ai") {
        cfg.ai.model = text(&ai, "model");
        if let Some(Value::Object(p)) = field(&ai, "providers") {
            for (id, v) in p.borrow().strings.iter() {
                cfg.ai.providers.push(ProviderConfig {
                    id: id.to_string(),
                    title: text(v, "title"),
                    url: text(v, "url"),
                    key_env: text(v, "keyEnv"),
                    client_id: text(v, "clientId"),
                    redirect_uri: text(v, "redirectUri"),
                    organization: text(v, "organization"),
                });
            }
        }
        cfg.ai.providers.sort_by(|a, b| a.id.cmp(&b.id));
    }
    let mut warnings = Vec::new();
    for (i, s) in items(&root, "shortcuts").iter().enumerate() {
        let kind_name = text(s, "kind");
        let Some(kind) = Kind::parse(&kind_name) else {
            warnings.push(format!("shortcut {}: unknown kind `{kind_name}` (url, open, command, shell, text, ai)", i + 1));
            continue;
        };
        let sc = Shortcut {
            keyword: text(s, "keyword"),
            name: text(s, "name"),
            kind,
            target: text(s, "target"),
            input: text(s, "input"),
            output: text(s, "output"),
            hotkey: text(s, "hotkey"),
            model: text(s, "model"),
            expand: matches!(field(s, "expand"), Some(Value::Bool(true))),
        };
        match check(&cfg, &sc, None) {
            Ok(()) => cfg.shortcuts.push(normalize(sc)),
            Err(e) => warnings.push(format!("shortcut {} ({}): {e}", i + 1, sc.keyword)),
        }
    }
    for (i, h) in items(&root, "hotkeys").iter().enumerate() {
        let b = HotkeyBinding { keys: text(h, "keys"), run: text(h, "run"), query: text(h, "query") };
        if b.keys.trim().is_empty() || b.run.trim().is_empty() {
            warnings.push(format!("hotkey {}: needs `keys` and `run`", i + 1));
            continue;
        }
        cfg.hotkeys.push(b);
    }
    Ok((cfg, warnings))
}

fn normalize(mut s: Shortcut) -> Shortcut {
    s.keyword = s.keyword.trim().to_string();
    if s.name.trim().is_empty() {
        s.name = s.keyword.clone();
    }
    if s.kind == Kind::Command && s.input.is_empty() {
        s.input = "{query}".into();
    }
    if s.kind == Kind::Shell && s.output.is_empty() {
        s.output = "show".into();
    }
    s
}

/// Whether `s` can be added to `cfg`; `replacing` is the keyword it replaces when editing.
pub fn check(cfg: &Config, s: &Shortcut, replacing: Option<&str>) -> Result<(), String> {
    check_keyword(cfg, &s.keyword, replacing)?;
    if s.target.trim().is_empty() {
        return Err("needs a target".into());
    }
    if s.kind == Kind::Shell && !s.output.is_empty() && !OUTPUTS.contains(&s.output.as_str()) {
        return Err(format!("output must be show, copy or none, not `{}`", s.output));
    }
    Ok(())
}

pub fn check_keyword(cfg: &Config, keyword: &str, replacing: Option<&str>) -> Result<(), String> {
    let k = keyword.trim();
    if k.is_empty() {
        return Err("needs a keyword".into());
    }
    if k.chars().any(char::is_whitespace) {
        return Err(format!("keyword `{k}` cannot contain spaces"));
    }
    if k.chars().count() > MAX_KEYWORD {
        return Err(format!("keyword `{k}` is longer than {MAX_KEYWORD} characters"));
    }
    let same = |other: &str| other.eq_ignore_ascii_case(k);
    if let Some(existing) = cfg.shortcuts.iter().find(|s| same(&s.keyword) && !replacing.is_some_and(same)) {
        return Err(format!("keyword `{k}` is already used by \"{}\"", existing.name));
    }
    Ok(())
}

/// Add `s`, or replace the shortcut keyed `replacing`.
pub fn upsert(cfg: &mut Config, s: Shortcut, replacing: Option<&str>) -> Result<(), String> {
    check(cfg, &s, replacing)?;
    let s = normalize(s);
    match replacing.and_then(|r| cfg.shortcuts.iter().position(|x| x.keyword.eq_ignore_ascii_case(r))) {
        Some(i) => cfg.shortcuts[i] = s,
        None => cfg.shortcuts.push(s),
    }
    Ok(())
}

pub fn remove(cfg: &mut Config, keyword: &str) -> bool {
    let before = cfg.shortcuts.len();
    cfg.shortcuts.retain(|s| !s.keyword.eq_ignore_ascii_case(keyword.trim()));
    cfg.shortcuts.len() != before
}

/// Bind `keys` to `run`, replacing an existing binding of the same keys.
pub fn bind(cfg: &mut Config, keys: &str, run: &str, query: &str) {
    let b = HotkeyBinding { keys: keys.trim().into(), run: run.trim().into(), query: query.into() };
    match cfg.hotkeys.iter().position(|h| h.keys.eq_ignore_ascii_case(&b.keys)) {
        Some(i) => cfg.hotkeys[i] = b,
        None => cfg.hotkeys.push(b),
    }
}

/// Remove the binding of `keys`, or the hotkey of the shortcut that has them.
pub fn unbind(cfg: &mut Config, keys: &str) -> bool {
    let before = cfg.hotkeys.len();
    cfg.hotkeys.retain(|h| !h.keys.eq_ignore_ascii_case(keys.trim()));
    let mut changed = cfg.hotkeys.len() != before;
    for s in cfg.shortcuts.iter_mut().filter(|s| s.hotkey.eq_ignore_ascii_case(keys.trim())) {
        s.hotkey.clear();
        changed = true;
    }
    changed
}

fn json_str(out: &mut String, s: &str) {
    out.push('"');
    tishlang_core::escape_json_string_into(out, s);
    out.push('"');
}

/// The file as written: two-space indent, one shortcut per line, fields in a fixed order and
/// empty optional fields left out.
pub fn to_json(cfg: &Config) -> String {
    let mut out = String::from("{\n");
    if !cfg.launcher.is_empty() {
        out.push_str("  \"launcher\": ");
        json_str(&mut out, &cfg.launcher);
        out.push_str(",\n");
    }
    out.push_str("  \"shortcuts\": [");
    for (i, s) in cfg.shortcuts.iter().enumerate() {
        out.push_str(if i == 0 { "\n    { " } else { ",\n    { " });
        let mut fields: Vec<(&str, &str)> =
            vec![("keyword", &s.keyword), ("name", &s.name), ("kind", s.kind.name()), ("target", &s.target)];
        if s.kind == Kind::Command && s.input != "{query}" {
            fields.push(("input", &s.input));
        }
        if s.kind == Kind::Shell && s.output != "show" {
            fields.push(("output", &s.output));
        }
        if !s.hotkey.is_empty() {
            fields.push(("hotkey", &s.hotkey));
        }
        if s.kind == Kind::Ai && !s.model.is_empty() {
            fields.push(("model", &s.model));
        }
        for (j, (k, v)) in fields.iter().enumerate() {
            if j > 0 {
                out.push_str(", ");
            }
            json_str(&mut out, k);
            out.push_str(": ");
            json_str(&mut out, v);
        }
        if s.kind == Kind::Text && s.expand {
            out.push_str(", \"expand\": true");
        }
        out.push_str(" }");
    }
    out.push_str(if cfg.shortcuts.is_empty() { "],\n" } else { "\n  ],\n" });
    out.push_str("  \"hotkeys\": [");
    for (i, h) in cfg.hotkeys.iter().enumerate() {
        out.push_str(if i == 0 { "\n    { \"keys\": " } else { ",\n    { \"keys\": " });
        json_str(&mut out, &h.keys);
        out.push_str(", \"run\": ");
        json_str(&mut out, &h.run);
        if !h.query.is_empty() {
            out.push_str(", \"query\": ");
            json_str(&mut out, &h.query);
        }
        out.push_str(" }");
    }
    out.push_str(if cfg.hotkeys.is_empty() { "]" } else { "\n  ]" });
    if !cfg.search.is_empty() {
        out.push_str(",\n  \"search\": ");
        json_str(&mut out, &cfg.search);
    }
    if cfg.ai != AiConfig::default() {
        out.push_str(",\n  \"ai\": { \"model\": ");
        json_str(&mut out, &cfg.ai.model);
        if !cfg.ai.providers.is_empty() {
            out.push_str(", \"providers\": {");
            for (i, p) in cfg.ai.providers.iter().enumerate() {
                out.push_str(if i == 0 { "\n    " } else { ",\n    " });
                json_str(&mut out, &p.id);
                out.push_str(": {");
                let fields = [("title", &p.title), ("url", &p.url), ("keyEnv", &p.key_env), ("clientId", &p.client_id), ("redirectUri", &p.redirect_uri), ("organization", &p.organization)];
                let mut first = true;
                for (k, v) in fields.iter().filter(|f| !f.1.is_empty()) {
                    out.push_str(if first { " " } else { ", " });
                    first = false;
                    json_str(&mut out, k);
                    out.push_str(": ");
                    json_str(&mut out, v);
                }
                out.push_str(" }");
            }
            out.push_str("\n  }");
        }
        out.push_str(" }");
    }
    out.push_str("\n}\n");
    out
}

/// Add or replace a provider by id.
pub fn set_provider(cfg: &mut Config, p: ProviderConfig) {
    match cfg.ai.providers.iter().position(|x| x.id == p.id) {
        Some(i) => cfg.ai.providers[i] = p,
        None => cfg.ai.providers.push(p),
    }
    cfg.ai.providers.sort_by(|a, b| a.id.cmp(&b.id));
}

/// Missing file: an empty config.
pub fn load(path: &Path) -> Result<(Config, Vec<String>), String> {
    match std::fs::read_to_string(path) {
        Ok(s) => parse(&s).map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok((Config::default(), Vec::new())),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// Write atomically (temp file, then rename), creating the folder.
pub fn save(path: &Path, cfg: &Config) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, to_json(cfg)).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

// ── Templates ───────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    Plain,
    /// Percent-encode inserted values (URL query or path component).
    Url,
    /// Single-quote inserted values for `/bin/sh`.
    Shell,
}

impl Encoding {
    pub fn for_kind(kind: Kind) -> Encoding {
        match kind {
            Kind::Url => Encoding::Url,
            Kind::Shell => Encoding::Shell,
            _ => Encoding::Plain,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Vars {
    pub query: String,
    pub clipboard: String,
    pub selection: String,
    pub date: String,
    pub time: String,
}

pub fn needs_query(template: &str) -> bool {
    template.contains("{query}") || template.contains("{}")
}

pub fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

pub fn expand(template: &str, vars: &Vars, enc: Encoding) -> String {
    let put = |v: &str| match enc {
        Encoding::Plain => v.to_string(),
        Encoding::Url => percent_encode(v),
        Encoding::Shell => shell_quote(v),
    };
    let mut out = String::with_capacity(template.len() + vars.query.len());
    let mut rest = template;
    while let Some(i) = rest.find('{') {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        let (value, len) = if tail.starts_with("{query}") {
            (Some(put(&vars.query)), 7)
        } else if tail.starts_with("{}") {
            (Some(put(&vars.query)), 2)
        } else if tail.starts_with("{clipboard}") {
            (Some(put(&vars.clipboard)), 11)
        } else if tail.starts_with("{selection}") {
            (Some(put(&vars.selection)), 11)
        } else if tail.starts_with("{date}") {
            (Some(vars.date.clone()), 6)
        } else if tail.starts_with("{time}") {
            (Some(vars.time.clone()), 6)
        } else {
            (None, 1)
        };
        match value {
            Some(v) => out.push_str(&v),
            None => out.push('{'),
        }
        rest = &tail[len..];
    }
    out.push_str(rest);
    out
}

#[repr(C)]
struct Tm {
    sec: i32,
    min: i32,
    hour: i32,
    mday: i32,
    mon: i32,
    year: i32,
    wday: i32,
    yday: i32,
    isdst: i32,
    gmtoff: i64,
    zone: *const std::ffi::c_char,
}

extern "C" {
    fn localtime_r(t: *const i64, out: *mut Tm) -> *mut Tm;
}

/// Local `(YYYY-MM-DD, HH:MM)`.
pub fn local_date_time() -> (String, String) {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    let mut tm = Tm { sec: 0, min: 0, hour: 0, mday: 0, mon: 0, year: 0, wday: 0, yday: 0, isdst: 0, gmtoff: 0, zone: std::ptr::null() };
    if unsafe { localtime_r(&now, &mut tm) }.is_null() {
        return (String::new(), String::new());
    }
    (format!("{:04}-{:02}-{:02}", tm.year + 1900, tm.mon + 1, tm.mday), format!("{:02}:{:02}", tm.hour, tm.min))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sc(keyword: &str, kind: Kind, target: &str) -> Shortcut {
        Shortcut {
            keyword: keyword.into(),
            name: String::new(),
            kind,
            target: target.into(),
            input: String::new(),
            output: String::new(),
            hotkey: String::new(),
            model: String::new(),
            expand: false,
        }
    }

    #[test]
    fn ai_snippets_and_search_round_trip() {
        let json = r#"{
          "shortcuts": [
            { "keyword": "fix", "kind": "ai", "target": "Fix the grammar: {selection}", "model": "hypery:gpt-5-mini" },
            { "keyword": ";sig", "kind": "text", "target": "Best,\nA", "expand": true }
          ],
          "hotkeys": [],
          "search": "https://duckduckgo.com/?q={query}",
          "ai": { "model": "ollama:llama3.2", "providers": { "work": { "url": "https://llm.example/v1", "keyEnv": "WORK_KEY" }, "hypery": { "clientId": "abc" } } }
        }"#;
        let (cfg, w) = parse(json).unwrap();
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(cfg.shortcuts[0].kind, Kind::Ai);
        assert_eq!(cfg.shortcuts[0].model, "hypery:gpt-5-mini");
        assert!(cfg.shortcuts[1].expand);
        assert_eq!(cfg.ai.model, "ollama:llama3.2");
        assert_eq!(cfg.ai.providers.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["hypery", "work"]);
        assert_eq!(cfg.ai.providers[1].key_env, "WORK_KEY");
        let written = to_json(&cfg);
        assert_eq!(parse(&written).unwrap().0, cfg, "{written}");
        assert!(written.contains("\"expand\": true"));
        let v = Vars { selection: "teh cat".into(), ..Default::default() };
        assert_eq!(expand(&cfg.shortcuts[0].target, &v, Encoding::Plain), "Fix the grammar: teh cat");
    }

    #[test]
    fn parses_skips_bad_entries_and_round_trips() {
        let json = r#"{
          "launcher": "cmd+space",
          "shortcuts": [
            { "keyword": "g", "name": "Google", "kind": "url", "target": "https://www.google.com/search?q={query}" },
            { "keyword": "dl", "kind": "open", "target": "~/Downloads", "hotkey": "ctrl+alt+d" },
            { "keyword": "G", "kind": "url", "target": "https://dup" },
            { "keyword": "x", "kind": "teleport", "target": "y" },
            { "keyword": "ip", "kind": "shell", "target": "curl -s ifconfig.me", "output": "copy" },
            { "keyword": "cc", "kind": "command", "target": "plugin:hello-list/change-case" }
          ],
          "hotkeys": [{ "keys": "cmd+shift+v", "run": "moo:clipboard" }, { "keys": "cmd+1" }]
        }"#;
        let (cfg, warnings) = parse(json).unwrap();
        assert_eq!(cfg.launcher, "cmd+space");
        let kws: Vec<&str> = cfg.shortcuts.iter().map(|s| s.keyword.as_str()).collect();
        assert_eq!(kws, ["g", "dl", "ip", "cc"]);
        assert_eq!(warnings.len(), 3, "{warnings:?}");
        assert!(warnings[0].contains("already used by \"Google\""));
        assert!(warnings[1].contains("unknown kind"));
        assert!(warnings[2].contains("needs `keys` and `run`"));
        assert_eq!(cfg.shortcuts[1].name, "dl", "name defaults to the keyword");
        assert_eq!(cfg.shortcuts[3].input, "{query}");
        assert_eq!(cfg.hotkeys, vec![HotkeyBinding { keys: "cmd+shift+v".into(), run: "moo:clipboard".into(), query: String::new() }]);

        let written = to_json(&cfg);
        let (again, w2) = parse(&written).unwrap();
        assert!(w2.is_empty());
        assert_eq!(again, cfg);
        assert!(written.contains("\"output\": \"copy\""));
        assert!(!written.contains("\"input\""), "default input is left out:\n{written}");
    }

    #[test]
    fn empty_and_malformed_files() {
        assert_eq!(parse("").unwrap().0, Config::default());
        assert!(parse("{ nope").is_err());
        assert!(parse("[1]").is_err());
        assert_eq!(to_json(&Config::default()), "{\n  \"shortcuts\": [],\n  \"hotkeys\": []\n}\n");
    }

    #[test]
    fn keywords_are_validated() {
        let mut cfg = Config::default();
        upsert(&mut cfg, sc("g", Kind::Url, "https://g/{query}"), None).unwrap();
        assert!(upsert(&mut cfg, sc("G", Kind::Url, "x"), None).unwrap_err().contains("already used"));
        assert!(upsert(&mut cfg, sc("a b", Kind::Url, "x"), None).unwrap_err().contains("spaces"));
        assert!(upsert(&mut cfg, sc("", Kind::Url, "x"), None).unwrap_err().contains("keyword"));
        assert!(upsert(&mut cfg, sc("t", Kind::Url, " "), None).unwrap_err().contains("target"));
        let mut edited = sc("G", Kind::Url, "https://google/{query}");
        edited.name = "Google".into();
        upsert(&mut cfg, edited, Some("g")).unwrap();
        assert_eq!(cfg.shortcuts.len(), 1);
        assert_eq!(cfg.shortcuts[0].keyword, "G");
        assert!(remove(&mut cfg, "g"));
        assert!(cfg.shortcuts.is_empty());
    }

    #[test]
    fn hotkey_bindings() {
        let mut cfg = Config::default();
        bind(&mut cfg, "cmd+shift+v", "moo:clipboard", "");
        bind(&mut cfg, "CMD+SHIFT+V", "moo:files", "");
        assert_eq!(cfg.hotkeys.len(), 1);
        assert_eq!(cfg.hotkeys[0].run, "moo:files");
        let mut s = sc("dl", Kind::Open, "~/Downloads");
        s.hotkey = "ctrl+alt+d".into();
        upsert(&mut cfg, s, None).unwrap();
        assert!(unbind(&mut cfg, "ctrl+alt+d"));
        assert!(cfg.shortcuts[0].hotkey.is_empty());
        assert!(unbind(&mut cfg, "cmd+shift+v"));
        assert!(!unbind(&mut cfg, "cmd+shift+v"));
    }

    #[test]
    fn templates_encode_per_kind() {
        let v = Vars { query: "rust & 'tish'".into(), clipboard: "a b".into(), date: "2026-10-03".into(), time: "17:20".into(), ..Default::default() };
        assert_eq!(expand("https://g.com/?q={query}", &v, Encoding::Url), "https://g.com/?q=rust%20%26%20%27tish%27");
        assert_eq!(expand("echo {query}", &v, Encoding::Shell), r"echo 'rust & '\''tish'\'''");
        assert_eq!(expand("{} | {clipboard} | {date} {time} | {other}", &v, Encoding::Plain), "rust & 'tish' | a b | 2026-10-03 17:20 | {other}");
        assert!(needs_query("x{}"));
        assert!(!needs_query("~/Downloads"));
        let (d, t) = local_date_time();
        assert_eq!((d.len(), t.len()), (10, 5));
    }
}
