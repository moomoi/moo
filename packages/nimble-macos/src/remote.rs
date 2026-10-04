//! Remote and local language models over the OpenAI-compatible chat API (`POST
//! {base}/chat/completions` with `stream: true`, `GET {base}/models`). Built in: Hypery (API key or
//! browser sign-in), OpenAI, and local Ollama and LM Studio servers; `ai.providers` in
//! shortcuts.json adds more or overrides these. A model is named `provider:model`.
//!
//! Credentials, first found: an API key in the Keychain (account = provider id), an OAuth token
//! (account `<id>.oauth`, refreshed when it expires), the provider's environment variable. Local
//! providers need none.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tishlang_core::{json_parse, ObjectMap, Value, VmRef};

use crate::shortcuts::AiConfig;
use crate::{bridge, http, keychain, oauth};

#[derive(Clone, Debug)]
pub struct Provider {
    pub id: String,
    pub title: String,
    pub url: String,
    pub key_env: String,
    /// Authorize and token paths on the URL's origin, for browser sign-in.
    pub oauth: Option<(&'static str, &'static str)>,
    /// Scopes to ask for at sign-in.
    pub scope: &'static str,
    pub client_id: String,
    pub local: bool,
    /// Appended to `{url}/models`.
    pub models_query: &'static str,
    /// The model list needs no credential.
    pub public_models: bool,
    /// Hypery's balance and user endpoints on the URL's origin.
    pub account: bool,
}

/// Nimble's OAuth app on hypery.ai (public client: PKCE, redirect `http://127.0.0.1/callback`,
/// scopes `ai:chat ai:models`). Empty until registered; config and `NIMBLE_HYPERY_CLIENT_ID` win.
const HYPERY_CLIENT_ID: &str = "";

fn builtins() -> Vec<Provider> {
    let p = |id: &str, title: &str, url: &str, key_env: &str, local: bool| Provider {
        id: id.into(),
        title: title.into(),
        url: url.into(),
        key_env: key_env.into(),
        oauth: None,
        scope: "",
        client_id: String::new(),
        local,
        models_query: "",
        public_models: false,
        account: false,
    };
    let mut hypery = p("hypery", "Hypery", "https://hypery.ai/v1", "HYPERY_API_KEY", false);
    hypery.oauth = Some(("/api/oauth/authorize", "/api/oauth/token"));
    hypery.scope = "ai:chat ai:models";
    hypery.models_query = "?category=chat&limit=1000";
    hypery.public_models = true;
    hypery.account = true;
    vec![
        hypery,
        p("openai", "OpenAI", "https://api.openai.com/v1", "OPENAI_API_KEY", false),
        p("ollama", "Ollama", "http://localhost:11434/v1", "", true),
        p("lmstudio", "LM Studio", "http://localhost:1234/v1", "", true),
    ]
}

fn is_local(url: &str) -> bool {
    let host = url.split("://").nth(1).unwrap_or("").split(['/', ':']).next().unwrap_or("");
    matches!(host, "localhost" | "127.0.0.1" | "[::1]")
}

/// Built-in providers with the config's overrides applied, then the config's own.
pub fn providers(cfg: &AiConfig) -> Vec<Provider> {
    let mut list = builtins();
    for c in &cfg.providers {
        let i = match list.iter().position(|p| p.id == c.id) {
            Some(i) => i,
            None => {
                list.push(Provider {
                    id: c.id.clone(),
                    title: c.id.clone(),
                    url: String::new(),
                    key_env: String::new(),
                    oauth: None,
                    scope: "",
                    client_id: String::new(),
                    local: false,
                    models_query: "",
                    public_models: false,
                    account: false,
                });
                list.len() - 1
            }
        };
        let p = &mut list[i];
        if !c.title.is_empty() {
            p.title = c.title.clone();
        }
        if !c.url.is_empty() {
            p.url = c.url.trim_end_matches('/').to_string();
            p.local = is_local(&p.url);
        }
        if !c.key_env.is_empty() {
            p.key_env = c.key_env.clone();
        }
        if !c.client_id.is_empty() {
            p.client_id = c.client_id.clone();
        }
    }
    for p in list.iter_mut().filter(|p| p.client_id.is_empty() && p.oauth.is_some()) {
        p.client_id = std::env::var(format!("NIMBLE_{}_CLIENT_ID", p.id.to_uppercase())).unwrap_or_default();
        if p.client_id.is_empty() && p.id == "hypery" {
            p.client_id = HYPERY_CLIENT_ID.into();
        }
    }
    list
}

/// `provider:model` -> its parts. `apple` (or empty) is Apple's on-device model.
pub fn split_model(m: &str) -> (String, String) {
    let m = m.trim();
    if m.is_empty() || m == "apple" {
        return ("apple".into(), String::new());
    }
    match m.split_once(':') {
        Some((p, rest)) => (p.to_string(), rest.to_string()),
        None => (m.to_string(), String::new()),
    }
}

fn origin(url: &str) -> &str {
    let start = url.find("://").map_or(0, |i| i + 3);
    match url[start..].find('/') {
        Some(i) => &url[..start + i],
        None => url,
    }
}

pub fn endpoints(p: &Provider) -> Option<oauth::Endpoints> {
    let (auth, token) = p.oauth?;
    let o = origin(&p.url);
    Some(oauth::Endpoints { authorize: format!("{o}{auth}"), token: format!("{o}{token}"), client_id: p.client_id.clone(), scope: p.scope.into() })
}

fn oauth_account(id: &str) -> String {
    format!("{id}.oauth")
}

/// What `credential` would find, without reading the Keychain.
pub fn credential_source(p: &Provider) -> &'static str {
    if keychain::has(&p.id) {
        "key"
    } else if keychain::has(&oauth_account(&p.id)) {
        "signed in"
    } else if !p.key_env.is_empty() && std::env::var(&p.key_env).is_ok_and(|v| !v.is_empty()) {
        "environment"
    } else {
        ""
    }
}

/// The API key or access token for `p`, if any. May refresh a token: worker threads only.
pub fn credential(p: &Provider) -> Result<Option<String>, String> {
    if keychain::has(&p.id) {
        if let Some(k) = keychain::get(&p.id) {
            return Ok(Some(k));
        }
    }
    let account = oauth_account(&p.id);
    if keychain::has(&account) {
        if let Some(t) = keychain::get(&account).and_then(|s| oauth::from_json(&s)) {
            if !oauth::stale(&t) {
                return Ok(Some(t.access));
            }
            if let (Some(ep), false) = (endpoints(p), t.refresh.is_empty()) {
                let fresh = oauth::refresh(&ep, &t.refresh).map_err(|e| format!("{}: sign in again ({e})", p.title))?;
                keychain::set(&account, &oauth::to_json(&fresh))?;
                return Ok(Some(fresh.access));
            }
        }
    }
    if !p.key_env.is_empty() {
        if let Ok(k) = std::env::var(&p.key_env) {
            if !k.is_empty() {
                return Ok(Some(k));
            }
        }
    }
    Ok(None)
}

fn no_credential(p: &Provider) -> String {
    let mut how = format!("{} needs an API key: `nimble ai key {} <key>`", p.title, p.id);
    if !p.key_env.is_empty() {
        how.push_str(&format!(" or ${}", p.key_env));
    }
    if p.oauth.is_some() {
        how.push_str(&format!(", or sign in with `nimble ai login {}`", p.id));
    }
    how
}

// ── Responses ───────────────────────────────────────────────────────────────

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
        _ => String::new(),
    }
}

fn list(v: &Value, key: &str) -> Vec<Value> {
    match get(v, key) {
        Some(Value::Array(a)) => a.borrow().clone(),
        _ => Vec::new(),
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// JSON text, assembled from streamed pieces.
    pub arguments: String,
}

/// A streamed chat completion, fed one server-sent-events line at a time.
#[derive(Debug, Default)]
pub struct Stream {
    pub text: String,
    pub calls: Vec<ToolCall>,
    pub finish: String,
    pub error: String,
    pub done: bool,
}

impl Stream {
    /// Feed one line; true when the text or tool calls changed.
    pub fn line(&mut self, line: &str) -> bool {
        let Some(data) = line.strip_prefix("data:") else { return false };
        let data = data.trim();
        if data == "[DONE]" {
            self.done = true;
            return false;
        }
        let Ok(v) = json_parse(data) else { return false };
        if let Some(e) = get(&v, "error") {
            self.error = error_text(&e);
            return false;
        }
        let mut changed = false;
        for choice in list(&v, "choices") {
            let finish = text(&choice, "finish_reason");
            if !finish.is_empty() {
                self.finish = finish;
            }
            let Some(delta) = get(&choice, "delta") else { continue };
            let piece = text(&delta, "content");
            if !piece.is_empty() {
                self.text.push_str(&piece);
                changed = true;
            }
            for tc in list(&delta, "tool_calls") {
                let i = match get(&tc, "index") {
                    Some(Value::Number(n)) => n as usize,
                    _ => self.calls.len().saturating_sub(1),
                };
                while self.calls.len() <= i {
                    self.calls.push(ToolCall::default());
                }
                let call = &mut self.calls[i];
                let id = text(&tc, "id");
                if !id.is_empty() {
                    call.id = id;
                }
                if let Some(f) = get(&tc, "function") {
                    let name = text(&f, "name");
                    if !name.is_empty() {
                        call.name = name;
                    }
                    call.arguments.push_str(&text(&f, "arguments"));
                }
                changed = true;
            }
        }
        changed
    }
}

fn error_text(e: &Value) -> String {
    match e {
        Value::String(s) => s.to_string(),
        _ => {
            let (code, msg) = (text(e, "code"), text(e, "message"));
            match (code.is_empty(), msg.is_empty()) {
                (false, false) => format!("{code}: {msg}"),
                (true, false) => msg,
                (false, true) => code,
                _ => "unknown error".into(),
            }
        }
    }
}

/// A readable message for a failed response.
pub fn error_message(status: u16, body: &str) -> String {
    let detail = json_parse(body)
        .ok()
        .map(|v| match get(&v, "error") {
            Some(e) => error_text(&e),
            None => error_text(&v),
        })
        .filter(|m| m != "unknown error")
        .unwrap_or_else(|| body.trim().chars().take(200).collect());
    let what = match status {
        401 => "the API key was rejected",
        402 => "out of credit",
        403 => "not allowed",
        404 => "model or endpoint not found",
        429 => "rate limited",
        500..=599 => "the server failed",
        _ => "request failed",
    };
    if detail.is_empty() {
        format!("HTTP {status}: {what}")
    } else {
        format!("HTTP {status}: {what} ({detail})")
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub owner: String,
    /// Tokens; 0 when not given.
    pub context: f64,
    /// Hypery's price band: Budget, Standard or Premium.
    pub tier: String,
}

/// A readable name: the gateway's display name without an "Author: " prefix, else the id's last part.
fn display_name(name: &str, id: &str) -> String {
    let name = name.trim();
    if name.is_empty() || name == id {
        return id.rsplit('/').next().unwrap_or(id).trim_start_matches('~').to_string();
    }
    match name.find(": ") {
        Some(i) if i <= 24 && !name[i + 2..].trim().is_empty() => name[i + 2..].trim().to_string(),
        _ => name.to_string(),
    }
}

/// The models in a `/models` response with their details, in the server's order (Hypery's is by
/// popularity), each id once.
pub fn parse_model_info(body: &str) -> Vec<ModelInfo> {
    let Ok(v) = json_parse(body) else { return Vec::new() };
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for m in list(&v, "data").iter().chain(list(&v, "models").iter()) {
        let id = text(m, "id");
        if id.is_empty() || !seen.insert(id.clone()) {
            continue;
        }
        let owner = text(m, "owned_by");
        let owner = if owner.is_empty() { id.split('/').next().filter(|_| id.contains('/')).unwrap_or("").trim_start_matches('~').to_string() } else { owner };
        out.push(ModelInfo {
            name: display_name(&text(m, "name"), &id),
            owner,
            context: text(m, "context_length").parse().unwrap_or(0.0),
            tier: text(m, "pricing_tier_label"),
            id,
        });
    }
    out
}

// ── Requests ────────────────────────────────────────────────────────────────

fn s(v: &str) -> Value {
    Value::String(v.into())
}

fn obj(pairs: Vec<(&str, Value)>) -> Value {
    let mut m = ObjectMap::default();
    for (k, v) in pairs {
        m.insert(Arc::from(k), v);
    }
    Value::object(m)
}

struct Out {
    request: u64,
    kind: &'static str,
    text: String,
    calls: Vec<ToolCall>,
    finish: String,
}

static QUEUE: Mutex<Vec<Out>> = Mutex::new(Vec::new());
static SCHEDULED: AtomicBool = AtomicBool::new(false);
static ACTIVE: Mutex<Option<HashMap<u64, u64>>> = Mutex::new(None);
static CANCELLED: Mutex<Option<HashSet<u64>>> = Mutex::new(None);

/// Queue an event for the main thread. Partials replace a waiting partial of the same request, so
/// a slow main thread gets the latest text rather than a backlog.
fn emit(o: Out) {
    let mut q = QUEUE.lock().unwrap_or_else(|e| e.into_inner());
    match q.last_mut() {
        Some(last) if last.request == o.request && last.kind == "partial" && o.kind == "partial" => *last = o,
        _ => q.push(o),
    }
    drop(q);
    if !SCHEDULED.swap(true, Ordering::AcqRel) {
        bridge::on_main(flush);
    }
}

fn flush() {
    SCHEDULED.store(false, Ordering::Release);
    let items = std::mem::take(&mut *QUEUE.lock().unwrap_or_else(|e| e.into_inner()));
    for o in items {
        let last = o.kind != "partial";
        let calls = o
            .calls
            .iter()
            .map(|c| obj(vec![("id", s(&c.id)), ("name", s(&c.name)), ("arguments", s(&c.arguments))]))
            .collect();
        let v = obj(vec![
            ("request", Value::Number(o.request as f64)),
            ("kind", s(o.kind)),
            ("text", s(&o.text)),
            ("calls", Value::Array(VmRef::new(calls))),
            ("finish", s(&o.finish)),
        ]);
        bridge::call(o.request, v, last);
    }
}

fn fail(request: u64, text: String) {
    ACTIVE.lock().unwrap_or_else(|e| e.into_inner()).as_mut().map(|m| m.remove(&request));
    emit(Out { request, kind: "error", text, calls: Vec::new(), finish: String::new() });
}

fn is_cancelled(request: u64) -> bool {
    CANCELLED.lock().unwrap_or_else(|e| e.into_inner()).as_mut().is_some_and(|c| c.remove(&request))
}

pub fn load_config() -> AiConfig {
    crate::shortcuts::config_path().and_then(|p| crate::shortcuts::load(&p).ok()).map(|(c, _)| c.ai).unwrap_or_default()
}

/// Stream a reply from `model` (`provider:model`) for `messages` (JSON array of chat messages);
/// `tools` is a JSON array of OpenAI function tools, or empty. `cb({ request, kind, text, calls,
/// finish })` runs on the main thread: kind "partial" (text so far), then "done", "error" or
/// "cancelled". Returns the request id.
pub fn chat(model: &str, messages: &str, tools: &str, cb: Option<Value>) -> u64 {
    let request = bridge::hold(cb);
    let (pid, name) = split_model(model);
    let provider = providers(&load_config()).into_iter().find(|p| p.id == pid);
    let (messages, tools) = (messages.to_string(), tools.trim().to_string());
    std::thread::spawn(move || {
        let Some(p) = provider else { return fail(request, format!("unknown AI provider `{pid}`")) };
        if name.is_empty() {
            return fail(request, format!("name a model: `{pid}:<model>` (see `nimble ai models`)"));
        }
        let key = match credential(&p) {
            Ok(Some(k)) => Some(k),
            Ok(None) if p.local => None,
            Ok(None) => return fail(request, no_credential(&p)),
            Err(e) => return fail(request, e),
        };
        let mut body = String::from("{\"model\":\"");
        tishlang_core::escape_json_string_into(&mut body, &name);
        body.push_str("\",\"stream\":true,\"messages\":");
        body.push_str(if messages.trim().is_empty() { "[]" } else { &messages });
        if !tools.is_empty() && tools != "[]" {
            body.push_str(",\"tools\":");
            body.push_str(&tools);
        }
        body.push('}');
        send(request, format!("{}/chat/completions", p.url), key, body, 0);
    });
    request
}

fn send(request: u64, url: String, key: Option<String>, body: String, attempt: u32) {
    if is_cancelled(request) {
        return emit(Out { request, kind: "cancelled", text: String::new(), calls: Vec::new(), finish: String::new() });
    }
    let mut req = http::Request::post_json(&url, body.clone()).header("Accept", "text/event-stream");
    req.timeout = 120.0;
    if let Some(k) = &key {
        req = req.header("Authorization", format!("Bearer {k}"));
    }
    let mut status = 0u16;
    let mut stream = Stream::default();
    let mut error_body = String::new();
    let retry = (url.clone(), key.clone(), body.clone());
    let handler: http::Handler = Box::new(move |e| match e {
        http::Event::Status(st) => status = st,
        http::Event::Line(l) if status == 200 => {
            if stream.line(&l) {
                emit(Out { request, kind: "partial", text: stream.text.clone(), calls: stream.calls.clone(), finish: String::new() });
            }
        }
        http::Event::Line(l) => {
            error_body.push_str(&l);
            error_body.push('\n');
        }
        http::Event::Done => {
            ACTIVE.lock().unwrap_or_else(|e| e.into_inner()).as_mut().map(|m| m.remove(&request));
            if status != 200 {
                let limited = status == 429 || error_body.contains("RATE_LIMITED");
                if limited && attempt < 2 {
                    let (url, key, body) = retry.clone();
                    std::thread::spawn(move || {
                        std::thread::sleep(Duration::from_millis(1500 << attempt));
                        send(request, url, key, body, attempt + 1);
                    });
                    return;
                }
                return fail(request, error_message(status, &error_body));
            }
            if !stream.error.is_empty() {
                return fail(request, stream.error.clone());
            }
            emit(Out {
                request,
                kind: "done",
                text: std::mem::take(&mut stream.text),
                calls: std::mem::take(&mut stream.calls),
                finish: std::mem::take(&mut stream.finish),
            });
        }
        http::Event::Failed(m) => fail(request, m),
        http::Event::Cancelled => {
            ACTIVE.lock().unwrap_or_else(|e| e.into_inner()).as_mut().map(|m| m.remove(&request));
            emit(Out { request, kind: "cancelled", text: std::mem::take(&mut stream.text), calls: Vec::new(), finish: String::new() });
        }
    });
    match http::start(&req, handler) {
        Some(id) => {
            ACTIVE.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(request, id);
        }
        None => fail(request, format!("invalid provider URL {url}")),
    }
}

pub fn cancel(request: u64) {
    let active = ACTIVE.lock().unwrap_or_else(|e| e.into_inner()).as_mut().and_then(|m| m.remove(&request));
    match active {
        Some(id) => http::cancel(id),
        None => {
            CANCELLED.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashSet::new).insert(request);
        }
    }
}

struct Listing {
    id: String,
    title: String,
    models: Vec<ModelInfo>,
    error: String,
}

/// `cb([{ provider, title, models: [{ id, name, owner, context, tier }], error }])` on the main
/// thread, for every provider but Apple's.
pub fn models(cb: Option<Value>) {
    let request = bridge::hold(cb);
    let list = providers(&load_config());
    std::thread::spawn(move || {
        let handles: Vec<_> = list
            .into_iter()
            .map(|p| {
                std::thread::spawn(move || {
                    let mut l = Listing { id: p.id.clone(), title: p.title.clone(), models: Vec::new(), error: String::new() };
                    let key = match credential(&p) {
                        Ok(k) => k,
                        Err(e) => {
                            l.error = e;
                            return l;
                        }
                    };
                    if key.is_none() && !p.local && !p.public_models {
                        l.error = "no API key".into();
                        return l;
                    }
                    let url = format!("{}/models{}", p.url, p.models_query);
                    let mut req = http::Request::get(&url);
                    req.timeout = if p.local { 2.0 } else { 20.0 };
                    if let Some(k) = key {
                        req = req.header("Authorization", format!("Bearer {k}"));
                    }
                    match http::fetch(&req) {
                        Ok((200, body)) => {
                            l.models = parse_model_info(&body);
                            if p.models_query.is_empty() {
                                l.models.sort_by(|a, b| a.id.cmp(&b.id));
                            }
                        }
                        Ok((st, body)) => l.error = error_message(st, &body),
                        Err(e) => l.error = if p.local { "not running".into() } else { e },
                    }
                    l
                })
            })
            .collect();
        let results: Vec<Listing> = handles.into_iter().filter_map(|h| h.join().ok()).collect();
        bridge::post(request, results, listings_value, true);
    });
}

fn listings_value(results: Vec<Listing>) -> Value {
    let rows = results
        .into_iter()
        .map(|l| {
            obj(vec![
                ("provider", s(&l.id)),
                ("title", s(&l.title)),
                (
                    "models",
                    Value::Array(VmRef::new(
                        l.models
                            .iter()
                            .map(|m| obj(vec![("id", s(&m.id)), ("name", s(&m.name)), ("owner", s(&m.owner)), ("context", Value::Number(m.context)), ("tier", s(&m.tier))]))
                            .collect(),
                    )),
                ),
                ("error", s(&l.error)),
            ])
        })
        .collect();
    Value::Array(VmRef::new(rows))
}

/// `[{ id, title, url, local, credential, oauth }]`, Apple's on-device model first.
pub fn providers_value() -> Value {
    let mut rows = vec![obj(vec![
        ("id", s("apple")),
        ("title", s("Apple on-device")),
        ("url", s("")),
        ("local", Value::Bool(true)),
        ("credential", s("")),
        ("oauth", Value::Bool(false)),
    ])];
    for p in providers(&load_config()) {
        rows.push(obj(vec![
            ("id", s(&p.id)),
            ("title", s(&p.title)),
            ("url", s(&p.url)),
            ("local", Value::Bool(p.local)),
            ("credential", s(credential_source(&p))),
            ("oauth", Value::Bool(p.oauth.is_some())),
            ("signIn", Value::Bool(p.oauth.is_some() && !p.client_id.is_empty())),
            ("account", Value::Bool(p.account)),
        ]));
    }
    Value::Array(VmRef::new(rows))
}

#[derive(Default)]
struct Account {
    provider: String,
    credential: String,
    email: String,
    name: String,
    /// Settings URLs on the provider's site.
    billing: String,
    keys: String,
    /// US dollars; -1 when unknown.
    balance: f64,
    month_spent: f64,
    month_limit: f64,
    error: String,
}

/// Credits as dollars (100 credits = $1).
fn dollars(credits: &str) -> f64 {
    credits.parse::<f64>().map(|c| c / 100.0).unwrap_or(-1.0)
}

fn fetch_account(p: &Provider) -> Account {
    let mut a = Account { provider: p.id.clone(), credential: credential_source(p).into(), balance: -1.0, month_spent: -1.0, month_limit: -1.0, ..Default::default() };
    let o = origin(&p.url).to_string();
    a.billing = format!("{o}/dashboard");
    a.keys = a.billing.clone();
    if !p.account {
        a.error = format!("{} has no account details", p.title);
        return a;
    }
    let key = match credential(p) {
        Ok(Some(k)) => k,
        Ok(None) => {
            a.error = "not signed in".into();
            return a;
        }
        Err(e) => {
            a.error = e;
            return a;
        }
    };
    let fetch_path = |path: &str| {
        let url = format!("{o}{path}");
        let mut req = http::Request::get(&url).header("Authorization", format!("Bearer {key}"));
        req.timeout = 15.0;
        http::fetch(&req)
    };
    if a.credential == "signed in" {
        if let Ok((200, body)) = fetch_path("/api/user/me") {
            if let Ok(v) = json_parse(&body) {
                a.email = text(&v, "email");
                a.name = text(&v, "name");
                let slug = text(&v, "slug");
                if !slug.is_empty() {
                    a.billing = format!("{o}/settings/{slug}/billing");
                    a.keys = format!("{o}/settings/{slug}/api-keys");
                }
            }
        }
    }
    match fetch_path("/api/wallet/balance") {
        Ok((200, body)) => match json_parse(&body).ok().and_then(|v| get(&v, "balance")) {
            Some(b) => {
                a.balance = dollars(&text(&b, "current"));
                a.month_spent = dollars(&text(&b, "monthlySpent"));
                a.month_limit = dollars(&text(&b, "monthlyLimit"));
            }
            None => a.error = "the balance reply had no balance".into(),
        },
        Ok((st, body)) => a.error = error_message(st, &body),
        Err(e) => a.error = e,
    }
    a
}

/// `cb({ provider, credential, email, name, billing, keys, balance, monthSpent, monthLimit, error })`
/// on the main thread. Amounts are US dollars, -1 when unknown.
pub fn account(provider: &str, cb: Option<Value>) {
    let request = bridge::hold(cb);
    let p = providers(&load_config()).into_iter().find(|p| p.id == provider);
    let name = provider.to_string();
    std::thread::spawn(move || {
        let a = match p {
            Some(p) => fetch_account(&p),
            None => Account { provider: name.clone(), balance: -1.0, month_spent: -1.0, month_limit: -1.0, error: format!("unknown AI provider `{name}`"), ..Default::default() },
        };
        bridge::post(
            request,
            a,
            |a| {
                obj(vec![
                    ("provider", s(&a.provider)),
                    ("credential", s(&a.credential)),
                    ("email", s(&a.email)),
                    ("name", s(&a.name)),
                    ("billing", s(&a.billing)),
                    ("keys", s(&a.keys)),
                    ("balance", Value::Number(a.balance)),
                    ("monthSpent", Value::Number(a.month_spent)),
                    ("monthLimit", Value::Number(a.month_limit)),
                    ("error", s(&a.error)),
                ])
            },
            true,
        );
    });
}

pub fn set_key(provider: &str, key: &str) -> Result<(), String> {
    if !providers(&load_config()).iter().any(|p| p.id == provider) {
        return Err(format!("unknown AI provider `{provider}`"));
    }
    if key.trim().is_empty() {
        keychain::delete(provider);
        return Ok(());
    }
    keychain::set(provider, key.trim())
}

pub fn logout(provider: &str) {
    keychain::delete(provider);
    keychain::delete(&oauth_account(provider));
}

static LOGIN_CANCEL: AtomicBool = AtomicBool::new(false);

struct LoginResult {
    error: String,
}

/// Browser sign-in for `provider`; `cb({ ok, error })` on the main thread.
pub fn login(provider: &str, cb: Option<Value>) {
    let request = bridge::hold(cb);
    let p = providers(&load_config()).into_iter().find(|p| p.id == provider);
    let name = provider.to_string();
    LOGIN_CANCEL.store(false, Ordering::Relaxed);
    std::thread::spawn(move || {
        let result = (|| {
            let p = p.ok_or_else(|| format!("unknown AI provider `{name}`"))?;
            let ep = endpoints(&p).ok_or_else(|| format!("{} has no browser sign-in; use an API key", p.title))?;
            if ep.client_id.is_empty() {
                return Err(format!(
                    "{} sign-in needs an OAuth client id: set ai.providers.{}.clientId in shortcuts.json or NIMBLE_{}_CLIENT_ID",
                    p.title,
                    p.id,
                    p.id.to_uppercase()
                ));
            }
            let tokens = oauth::login(&ep, |url| {
                let url = url.to_string();
                bridge::on_main(move || {
                    crate::mac::launch(&url);
                });
            }, &LOGIN_CANCEL)?;
            keychain::set(&oauth_account(&p.id), &oauth::to_json(&tokens))
        })();
        bridge::post(request, LoginResult { error: result.err().unwrap_or_default() }, |r| obj(vec![("ok", Value::Bool(r.error.is_empty())), ("error", s(&r.error))]), true);
    });
}

pub fn cancel_login() {
    LOGIN_CANCEL.store(true, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streams_text_and_tool_calls() {
        let mut st = Stream::default();
        assert!(!st.line(": keep-alive"));
        assert!(st.line(r#"data: {"choices":[{"delta":{"role":"assistant","content":"Hel"}}]}"#));
        assert!(st.line(r#"data: {"choices":[{"delta":{"content":"lo"}}]}"#));
        assert!(st.line(r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"open_app","arguments":"{\"na"}}]}}]}"#));
        assert!(st.line(r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"me\":\"Notes\"}"}}]}}]}"#));
        assert!(!st.line(r#"data: {"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#));
        st.line("data: [DONE]");
        assert_eq!(st.text, "Hello");
        assert_eq!(st.calls, vec![ToolCall { id: "call_1".into(), name: "open_app".into(), arguments: r#"{"name":"Notes"}"#.into() }]);
        assert_eq!(st.finish, "tool_calls");
        assert!(st.done);

        let mut bad = Stream::default();
        bad.line(r#"data: {"error":{"code":"INSUFFICIENT_CREDITS","message":"Top up"}}"#);
        assert_eq!(bad.error, "INSUFFICIENT_CREDITS: Top up");
    }

    #[test]
    fn errors_models_and_names() {
        assert_eq!(error_message(401, r#"{"error":{"message":"Invalid key"}}"#), "HTTP 401: the API key was rejected (Invalid key)");
        assert_eq!(error_message(429, r#"{"error":{"code":"RATE_LIMITED","message":"slow down"}}"#), "HTTP 429: rate limited (RATE_LIMITED: slow down)");
        assert_eq!(error_message(502, "Bad Gateway"), "HTTP 502: the server failed (Bad Gateway)");
        let ids = |body: &str| parse_model_info(body).into_iter().map(|m| m.id).collect::<Vec<_>>();
        assert_eq!(ids(r#"{"object":"list","data":[{"id":"b"},{"id":"a"},{"id":"b"}]}"#), ["b", "a"]);
        assert_eq!(split_model("hypery:openai/gpt-5"), ("hypery".into(), "openai/gpt-5".into()));
        assert_eq!(split_model("ollama:llama3.2:3b"), ("ollama".into(), "llama3.2:3b".into()));
        assert_eq!(split_model(""), ("apple".into(), String::new()));
        assert_eq!(origin("https://hypery.ai/v1"), "https://hypery.ai");
        assert!(is_local("http://127.0.0.1:8080/v1") && is_local("http://localhost:11434/v1") && !is_local("https://hypery.ai/v1"));
    }

    #[test]
    fn config_overrides_and_adds_providers() {
        let cfg = AiConfig {
            model: String::new(),
            providers: vec![
                crate::shortcuts::ProviderConfig { id: "hypery".into(), url: "http://127.0.0.1:9/v1/".into(), client_id: "cid".into(), ..Default::default() },
                crate::shortcuts::ProviderConfig { id: "work".into(), url: "https://llm.example/v1".into(), key_env: "WORK_KEY".into(), ..Default::default() },
            ],
        };
        let ps = providers(&cfg);
        let h = ps.iter().find(|p| p.id == "hypery").unwrap();
        assert_eq!((h.url.as_str(), h.local, h.client_id.as_str()), ("http://127.0.0.1:9/v1", true, "cid"));
        let ep = endpoints(h).unwrap();
        assert_eq!(ep.token, "http://127.0.0.1:9/api/oauth/token");
        assert_eq!(ep.scope, "ai:chat ai:models", "Hypery's authorize requires a scope");
        let w = ps.iter().find(|p| p.id == "work").unwrap();
        assert_eq!((w.title.as_str(), w.key_env.as_str(), w.local), ("work", "WORK_KEY", false));
    }

    #[test]
    fn model_details_from_hypery() {
        let body = r#"{"object":"list","data":[
            {"id":"~anthropic/claude-sonnet-latest","owned_by":"anthropic","name":"Anthropic: Claude Sonnet Latest","pricing_tier_label":"Standard","context_length":1000000},
            {"id":"x-ai/grok-4.3","owned_by":"x-ai","name":"SpaceXAI: Grok 4.3","pricing_tier_label":"Standard","context_length":1000000},
            {"id":"x-ai/grok-4.3","owned_by":"x-ai","name":"Grok 4.3 (again)"},
            {"id":"google/gemma-3-12b-it","owned_by":"google","name":"google/gemma-3-12b-it","pricing_tier_label":"Budget"},
            {"id":"anthropic/claude-sonnet-5","name":"Claude Sonnet 5"}
        ],"has_more":false}"#;
        let m = parse_model_info(body);
        let names: Vec<_> = m.iter().map(|x| x.name.as_str()).collect();
        assert_eq!(names, ["Claude Sonnet Latest", "Grok 4.3", "gemma-3-12b-it", "Claude Sonnet 5"]);
        assert_eq!((m[0].owner.as_str(), m[0].context, m[0].tier.as_str()), ("anthropic", 1_000_000.0, "Standard"));
        assert_eq!(m[3].owner, "anthropic", "the owner falls back to the id's author");
        assert_eq!(display_name("Ratio: 1:2 Mixer Deluxe Model Name Long", "x/y"), "1:2 Mixer Deluxe Model Name Long");
        assert_eq!(display_name("A very long model name that is long: with colon", "x/y"), "A very long model name that is long: with colon");
    }

    #[test]
    fn account_balance_and_user() {
        let base = crate::http::tests::serve(|line, headers, _b| {
            let authed = headers.to_ascii_lowercase().contains("authorization: bearer ak_test");
            if !authed {
                (401, "application/json", vec![r#"{"error":{"code":"UNAUTHENTICATED","message":"Unauthorized"}}"#.into()])
            } else if line.starts_with("GET /api/wallet/balance") {
                (200, "application/json", vec![r#"{"success":true,"balance":{"current":1234,"reserved":0,"monthlySpent":56,"monthlyLimit":100000}}"#.into()])
            } else {
                (404, "text/plain", vec![])
            }
        });
        let mut p = builtins().remove(0);
        p.id = format!("hypery-test-{}", std::process::id());
        p.url = format!("{base}/v1");
        p.key_env = format!("NIMBLE_TEST_ACCOUNT_KEY_{}", std::process::id());
        std::env::set_var(&p.key_env, "ak_test");
        let a = fetch_account(&p);
        assert_eq!(a.error, "");
        assert_eq!((a.balance, a.month_spent, a.month_limit), (12.34, 0.56, 1000.0));
        assert_eq!(a.credential, "environment");
        std::env::set_var(&p.key_env, "ak_wrong");
        let bad = fetch_account(&p);
        assert!(bad.error.starts_with("HTTP 401"), "{}", bad.error);
        std::env::remove_var(&p.key_env);
    }
}
