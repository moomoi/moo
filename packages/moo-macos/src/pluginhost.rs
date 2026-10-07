//! The `moo` object a Tier A plugin's VM gets: the host services a plugin may use, all generic.
//!
//!   moo.fetch({ method, url, headers, body, form }, cb)   cb({ status, body, error })
//!   moo.store.get(key) / set(key, text) / remove(key)       plain per-plugin storage
//!   moo.secret.get(key) / set(key, text) / remove(key)      the login keychain
//!   moo.signIn({ authorize, token, clientId, scope, params }, cb)
//!                                                           cb({ ok, accessToken, refreshToken, expiresAt, error })
//!   moo.cancelSignIn()
//!   moo.refresh()                                           ask the shell to ask again (list, suggestions)
//!   moo.notify(text)                                        a message in the shell's status line
//!   moo.log(text)
//!
//! Network access (fetch and sign-in URLs) is limited to https hosts the plugin's manifest names in
//! `permissions.network` (a host covers its subdomains). Callbacks run on the main thread under the
//! plugin's call budget.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tishlang_core::{json_parse, ObjectMap, Value};

use crate::{bridge, http, keychain, oauth};

/// The relay page registered as the redirect for browser sign-in (web/ in this repo).
const RELAY: &str = "https://moo.moi/callback";

#[derive(Default)]
pub struct Host {
    pub id: String,
    pub network: Vec<String>,
    /// This plugin's pending sign-in, if any: `cancelSignIn` stops only its own.
    pub sign_in_cancel: Option<Arc<AtomicBool>>,
}

pub type Shared = Arc<Mutex<Host>>;


thread_local! {
    static REFRESH: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    static REFRESH_QUEUED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The shell's callback for `moo.refresh()` (main thread).
pub fn on_refresh(cb: Option<Value>) {
    let old = REFRESH.with(|r| r.replace(bridge::hold(cb)));
    if old != 0 {
        bridge::release(old);
    }
}

fn get(v: &Value, key: &str) -> Option<Value> {
    match v {
        Value::Object(o) => o.borrow().strings.get(key).cloned(),
        _ => None,
    }
}

fn text(v: &Value, key: &str) -> String {
    match get(v, key) {
        Some(Value::String(s)) => s.to_string(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        _ => String::new(),
    }
}

fn pairs(v: Option<Value>) -> Vec<(String, String)> {
    match v {
        Some(Value::Object(o)) => o.borrow().strings.iter().map(|(k, v)| (k.to_string(), value_text(v))).collect(),
        _ => Vec::new(),
    }
}

fn value_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.to_string(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        _ => String::new(),
    }
}

fn obj(pairs: Vec<(&str, Value)>) -> Value {
    let mut m = ObjectMap::default();
    for (k, v) in pairs {
        m.insert(Arc::from(k), v);
    }
    Value::object(m)
}

fn s(v: &str) -> Value {
    Value::String(v.into())
}

/// `permissions.network` from a manifest.
pub fn network_permissions(manifest: &Value) -> Vec<String> {
    match get(manifest, "permissions").and_then(|p| get(&p, "network")) {
        Some(Value::Array(a)) => a.borrow().iter().map(value_text).map(|h| h.trim().to_lowercase()).filter(|h| !h.is_empty()).collect(),
        _ => Vec::new(),
    }
}

/// An https URL on a host in `allowed` (or a subdomain of one).
pub fn allowed(url: &str, allowed: &[String]) -> bool {
    let Some(rest) = url.strip_prefix("https://") else { return false };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.contains('@') {
        return false;
    }
    let host = authority.split(':').next().unwrap_or("").to_lowercase();
    !host.is_empty() && allowed.iter().any(|a| host == *a || host.ends_with(&format!(".{a}")))
}

fn data_dir() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("MOO_PLUGIN_DATA") {
        return Some(PathBuf::from(p));
    }
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support/Moo/plugins"))
}

/// A plugin id that is safe as a file name.
fn safe_id(id: &str) -> Option<String> {
    let ok = !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    ok.then(|| id.to_string())
}

fn store_path(id: &str) -> Option<PathBuf> {
    Some(data_dir()?.join(format!("{}.json", safe_id(id)?)))
}

fn load_store(id: &str) -> BTreeMap<String, String> {
    let Some(text) = store_path(id).and_then(|p| std::fs::read_to_string(p).ok()) else { return BTreeMap::new() };
    match json_parse(&text) {
        Ok(Value::Object(o)) => o.borrow().strings.iter().map(|(k, v)| (k.to_string(), value_text(v))).collect(),
        _ => BTreeMap::new(),
    }
}

fn save_store(id: &str, map: &BTreeMap<String, String>) -> bool {
    let Some(path) = store_path(id) else { return false };
    let mut out = String::from("{");
    for (i, (k, v)) in map.iter().enumerate() {
        out.push_str(if i == 0 { "\n  \"" } else { ",\n  \"" });
        tishlang_core::escape_json_string_into(&mut out, k);
        out.push_str("\": \"");
        tishlang_core::escape_json_string_into(&mut out, v);
        out.push('"');
    }
    out.push_str("\n}\n");
    let tmp = path.with_extension("json.tmp");
    path.parent().is_some_and(|d| std::fs::create_dir_all(d).is_ok()) && std::fs::write(&tmp, out).is_ok() && std::fs::rename(&tmp, &path).is_ok()
}

/// The Keychain account for a plugin secret, or None when the id or key isn't `[A-Za-z0-9_-]+`.
/// Neither may contain the `.` separator, so `("a.b", "c")` and `("a", "b.c")` can't collide.
fn secret_account(id: &str, key: &str) -> Option<String> {
    Some(format!("plugin.{}.{}", safe_id(id)?, safe_id(key)?))
}

fn arg(args: &[Value], i: usize) -> String {
    args.get(i).map(value_text).unwrap_or_default()
}

/// Wrap a plugin callback so it runs under the plugin's call budget.
fn callback(args: &[Value], i: usize, what: String) -> Option<Value> {
    match args.get(i) {
        Some(f @ Value::Function(_)) => Some(crate::vmplug::budgeted(f.clone(), what)),
        _ => None,
    }
}

struct Fetched {
    status: u16,
    body: String,
    error: String,
}

fn fetched_value(f: Fetched) -> Value {
    obj(vec![("status", Value::Number(f.status as f64)), ("body", s(&f.body)), ("error", s(&f.error))])
}

fn fetch(host: &Shared, args: &[Value]) {
    let (id, network) = {
        let h = host.lock().unwrap_or_else(|e| e.into_inner());
        (h.id.clone(), h.network.clone())
    };
    let request = bridge::hold(callback(args, 1, format!("{id}: fetch callback")));
    let req = args.first().cloned().unwrap_or(Value::Null);
    let url = text(&req, "url");
    let method = match text(&req, "method").to_uppercase() {
        m if m.is_empty() => "GET".to_string(),
        m => m,
    };
    let headers = pairs(get(&req, "headers"));
    let form = pairs(get(&req, "form"));
    let body = text(&req, "body");
    if !allowed(&url, &network) {
        let error = format!("{id} may not reach {url}: add its host to permissions.network");
        return bridge::post(request, Fetched { status: 0, body: String::new(), error }, fetched_value, true);
    }
    std::thread::spawn(move || {
        let method: &'static str = match method.as_str() {
            "POST" => "POST",
            "PUT" => "PUT",
            "PATCH" => "PATCH",
            "DELETE" => "DELETE",
            _ => "GET",
        };
        let mut req = http::Request { method, url: &url, headers: Vec::new(), body: Vec::new(), timeout: 30.0, follow_redirects: false };
        for (k, v) in &headers {
            req.headers.push((k.as_str(), v.clone()));
        }
        if !form.is_empty() {
            let f: Vec<(&str, &str)> = form.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
            req.body = http::form(&f);
            if !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("content-type")) {
                req.headers.push(("Content-Type", "application/x-www-form-urlencoded".into()));
            }
        } else {
            req.body = body.into_bytes();
        }
        let out = match http::fetch(&req) {
            Ok((status, body)) => Fetched { status, body, error: String::new() },
            Err(error) => Fetched { status: 0, body: String::new(), error },
        };
        bridge::post(request, out, fetched_value, true);
    });
}

struct SignedIn {
    tokens: Option<oauth::Tokens>,
    error: String,
}

fn signed_in_value(r: SignedIn) -> Value {
    let t = r.tokens.unwrap_or(oauth::Tokens { access: String::new(), refresh: String::new(), expires_at: 0.0 });
    obj(vec![
        ("ok", Value::Bool(r.error.is_empty())),
        ("accessToken", s(&t.access)),
        ("refreshToken", s(&t.refresh)),
        ("expiresAt", Value::Number(t.expires_at)),
        ("error", s(&r.error)),
    ])
}

fn sign_in(host: &Shared, args: &[Value]) {
    let cancel = Arc::new(AtomicBool::new(false));
    let (id, network) = {
        let mut h = host.lock().unwrap_or_else(|e| e.into_inner());
        // A new sign-in replaces this plugin's previous one: stop that one first.
        if let Some(old) = h.sign_in_cancel.replace(cancel.clone()) {
            old.store(true, Ordering::Relaxed);
        }
        (h.id.clone(), h.network.clone())
    };
    let request = bridge::hold(callback(args, 1, format!("{id}: signIn callback")));
    let o = args.first().cloned().unwrap_or(Value::Null);
    let ep = oauth::Endpoints {
        authorize: text(&o, "authorize"),
        token: text(&o, "token"),
        client_id: text(&o, "clientId"),
        scope: text(&o, "scope"),
        relay: RELAY.into(),
        extra: pairs(get(&o, "params")),
    };
    if !allowed(&ep.authorize, &network) || !allowed(&ep.token, &network) {
        let error = format!("{id} may not sign in through {}: add its host to permissions.network", ep.authorize);
        return bridge::post(request, SignedIn { tokens: None, error }, signed_in_value, true);
    }
    std::thread::spawn(move || {
        let r = oauth::login(
            &ep,
            |url| {
                let url = url.to_string();
                bridge::on_main(move || {
                    crate::mac::launch(&url);
                });
            },
            &cancel,
        );
        let out = match r {
            Ok(t) => SignedIn { tokens: Some(t), error: String::new() },
            Err(error) => SignedIn { tokens: None, error },
        };
        bridge::post(request, out, signed_in_value, true);
    });
}

fn refresh() {
    if REFRESH_QUEUED.with(|q| q.replace(true)) {
        return;
    }
    bridge::on_main(|| {
        REFRESH_QUEUED.with(|q| q.set(false));
        let id = REFRESH.with(|r| r.get());
        if id != 0 {
            bridge::call(id, Value::Null, false);
        }
    });
}

/// The shell's hook again, with a message for its status line (a send finished, a sign-in failed).
fn notify(text: String) {
    bridge::on_main(move || {
        let id = REFRESH.with(|r| r.get());
        if id != 0 {
            bridge::call(id, Value::String(text.into()), false);
        }
    });
}

fn native(host: &Shared, f: fn(&Shared, &[Value]) -> Value) -> Value {
    let host = host.clone();
    Value::native(move |args: &[Value]| f(&host, args))
}

fn id_of(host: &Shared) -> String {
    host.lock().unwrap_or_else(|e| e.into_inner()).id.clone()
}

/// The `moo` global for one plugin.
pub fn object(host: Shared) -> Value {
    let store = obj(vec![
        ("get", native(&host, |h, a| s(load_store(&id_of(h)).get(&arg(a, 0)).map_or("", |v| v.as_str())))),
        (
            "set",
            native(&host, |h, a| {
                let id = id_of(h);
                let mut m = load_store(&id);
                m.insert(arg(a, 0), arg(a, 1));
                Value::Bool(save_store(&id, &m))
            }),
        ),
        (
            "remove",
            native(&host, |h, a| {
                let id = id_of(h);
                let mut m = load_store(&id);
                let had = m.remove(&arg(a, 0)).is_some();
                Value::Bool(had && save_store(&id, &m))
            }),
        ),
    ]);
    let secret = obj(vec![
        (
            "get",
            native(&host, |h, a| {
                s(&secret_account(&id_of(h), &arg(a, 0)).and_then(|acct| keychain::get(&acct)).unwrap_or_default())
            }),
        ),
        (
            "set",
            native(&host, |h, a| {
                Value::Bool(secret_account(&id_of(h), &arg(a, 0)).is_some_and(|acct| keychain::set(&acct, &arg(a, 1)).is_ok()))
            }),
        ),
        (
            "remove",
            native(&host, |h, a| Value::Bool(secret_account(&id_of(h), &arg(a, 0)).is_some_and(|acct| keychain::delete(&acct)))),
        ),
    ]);
    obj(vec![
        (
            "fetch",
            native(&host, |h, a| {
                fetch(h, a);
                Value::Null
            }),
        ),
        ("store", store),
        ("secret", secret),
        (
            "signIn",
            native(&host, |h, a| {
                sign_in(h, a);
                Value::Null
            }),
        ),
        (
            "cancelSignIn",
            native(&host, |h, _| {
                if let Some(c) = h.lock().unwrap_or_else(|e| e.into_inner()).sign_in_cancel.take() {
                    c.store(true, Ordering::Relaxed);
                }
                Value::Null
            }),
        ),
        (
            "refresh",
            native(&host, |_, _| {
                refresh();
                Value::Null
            }),
        ),
        (
            "notify",
            native(&host, |_, a| {
                notify(arg(a, 0));
                Value::Null
            }),
        ),
        (
            "log",
            native(&host, |h, a| {
                // Plugins log what they like (tokens included), so it only goes out with MOO_DEBUG.
                crate::mac::debug_log(&format!("plugin {}: {}", id_of(h), arg(a, 0)));
                Value::Null
            }),
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_accounts_cannot_collide_or_escape() {
        assert_eq!(secret_account("slack", "token").as_deref(), Some("plugin.slack.token"));
        // "a.b"+"c" and "a"+"b.c" used to both be plugin.a.b.c.
        assert_eq!(secret_account("a", "b.c"), None);
        assert_eq!(secret_account("a.b", "c"), None);
        assert_eq!(secret_account("slack", ""), None);
        assert_eq!(secret_account("../x", "token"), None);
    }

    #[test]
    fn network_is_limited_to_declared_https_hosts() {
        let hosts = vec!["slack.com".to_string()];
        assert!(allowed("https://slack.com/api/chat.postMessage", &hosts));
        assert!(allowed("https://files.slack.com/x", &hosts));
        assert!(allowed("https://SLACK.com:443/api", &hosts));
        assert!(!allowed("http://slack.com/api", &hosts), "https only");
        assert!(!allowed("https://evilslack.com/", &hosts));
        assert!(!allowed("https://slack.com.evil.io/", &hosts));
        assert!(!allowed("https://slack.com@evil.io/", &hosts));
        assert!(!allowed("https://example.com/", &[]));
    }

    #[test]
    fn manifest_permissions() {
        let m = json_parse(r#"{"id":"slack","permissions":{"network":["Slack.com"," "]}}"#).unwrap();
        assert_eq!(network_permissions(&m), ["slack.com"]);
        assert!(network_permissions(&json_parse(r#"{"id":"x"}"#).unwrap()).is_empty());
    }

    #[test]
    fn store_keeps_text_per_plugin() {
        let dir = std::env::temp_dir().join(format!("moo-plugin-store-{}", std::process::id()));
        std::env::set_var("MOO_PLUGIN_DATA", &dir);
        let mut m = BTreeMap::new();
        m.insert("cache".to_string(), "{\"a\": \"line\\nbreak\"}".to_string());
        assert!(save_store("slack", &m));
        assert_eq!(load_store("slack"), m);
        assert!(load_store("other").is_empty());
        assert!(store_path("../evil").is_none());
        std::fs::remove_dir_all(&dir).ok();
        std::env::remove_var("MOO_PLUGIN_DATA");
    }
}
