//! OAuth 2.0 sign-in with PKCE (S256) through a loopback redirect: Moo listens on
//! `127.0.0.1:<any port>/callback`, opens the provider's authorize page in the browser, takes the
//! code from the redirect (directly, or forwarded by a relay page such as moo.moi/callback) and
//! exchanges it for tokens. Refresh tokens rotate on use.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use tishlang_core::{json_parse, Value};

use crate::http;

pub struct Endpoints {
    pub authorize: String,
    pub token: String,
    pub client_id: String,
    pub scope: String,
    /// The registered redirect when it is a web page rather than the loopback address: it reads
    /// the listener's port from `state` (`<port>.<nonce>`) and redirects the browser, code and all,
    /// to `http://127.0.0.1:<port>/callback`. Empty: the loopback address is the redirect.
    pub relay: String,
    /// More authorize parameters (Slack names its user scopes `user_scope`, say).
    pub extra: Vec<(String, String)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Tokens {
    pub access: String,
    pub refresh: String,
    /// Unix seconds; 0 when the server did not say.
    pub expires_at: f64,
}

#[cfg(target_os = "macos")]
extern "C" {
    fn CC_SHA256(data: *const u8, len: u32, md: *mut u8) -> *mut u8;
    fn getentropy(buf: *mut u8, len: usize) -> i32;
}

#[cfg(target_os = "macos")]
pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    unsafe { CC_SHA256(data.as_ptr(), data.len() as u32, out.as_mut_ptr()) };
    out
}

#[cfg(windows)]
pub fn sha256(data: &[u8]) -> [u8; 32] {
    use sha2::Digest;
    sha2::Sha256::digest(data).into()
}

/// Fill `b` from the system's secure random source; false on failure.
#[cfg(target_os = "macos")]
fn fill_random(b: &mut [u8]) -> bool {
    unsafe { getentropy(b.as_mut_ptr(), b.len()) == 0 }
}

#[cfg(windows)]
fn fill_random(b: &mut [u8]) -> bool {
    use windows::Win32::Security::Cryptography::{BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG};
    unsafe { BCryptGenRandom(None, b, BCRYPT_USE_SYSTEM_PREFERRED_RNG).is_ok() }
}

pub fn base64url(data: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for i in 0..chunk.len() + 1 {
            out.push(A[(n >> (18 - 6 * i) & 63) as usize] as char);
        }
    }
    out
}

/// `n` random bytes, or an error: a failed `getentropy` must not leave a predictable all-zero
/// verifier or state.
fn random(n: usize) -> Result<Vec<u8>, String> {
    let mut b = vec![0u8; n];
    if !fill_random(&mut b) {
        return Err("couldn't get random bytes for sign-in".into());
    }
    Ok(b)
}

/// `(verifier, challenge)` for PKCE S256.
pub fn pkce() -> Result<(String, String), String> {
    let verifier = base64url(&random(32)?);
    let challenge = base64url(&sha256(verifier.as_bytes()));
    Ok((verifier, challenge))
}

fn now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

fn field(v: &Value, key: &str) -> String {
    match v {
        Value::Object(o) => match o.borrow().strings.get(key) {
            Some(Value::String(s)) => s.to_string(),
            Some(Value::Number(n)) => n.to_string(),
            _ => String::new(),
        },
        _ => String::new(),
    }
}

fn parse_tokens(body: &str, old_refresh: &str) -> Result<Tokens, String> {
    let v = json_parse(body).map_err(|_| format!("token endpoint sent something other than JSON: {}", body.chars().take(120).collect::<String>()))?;
    let access = field(&v, "access_token");
    if access.is_empty() {
        let err = [field(&v, "error_description"), field(&v, "error")].into_iter().find(|e| !e.is_empty());
        return Err(err.unwrap_or_else(|| format!("no access token: {}", body.chars().take(160).collect::<String>())));
    }
    let refresh = field(&v, "refresh_token");
    let expires_in: f64 = field(&v, "expires_in").parse().unwrap_or(0.0);
    Ok(Tokens {
        access,
        refresh: if refresh.is_empty() { old_refresh.to_string() } else { refresh },
        expires_at: if expires_in > 0.0 { now() + expires_in } else { 0.0 },
    })
}

fn token_request(ep: &Endpoints, params: &[(&str, &str)], old_refresh: &str) -> Result<Tokens, String> {
    let req = http::Request {
        method: "POST",
        url: &ep.token,
        headers: vec![("Content-Type", "application/x-www-form-urlencoded".into()), ("Accept", "application/json".into())],
        body: http::form(params),
        timeout: 30.0,
        // Tokens go only to the endpoint that was checked (a plugin's may only be a declared host).
        follow_redirects: false,
    };
    let (status, body) = http::fetch(&req)?;
    if status >= 400 {
        let msg = json_parse(&body).map(|v| field(&v, "error_description")).unwrap_or_default();
        return Err(format!("sign-in failed (HTTP {status}){}", if msg.is_empty() { String::new() } else { format!(": {msg}") }));
    }
    parse_tokens(&body, old_refresh)
}

pub fn refresh(ep: &Endpoints, refresh_token: &str) -> Result<Tokens, String> {
    token_request(ep, &[("grant_type", "refresh_token"), ("refresh_token", refresh_token), ("client_id", &ep.client_id)], refresh_token)
}

pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => match std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                Some(v) => {
                    out.push(v);
                    i += 2;
                }
                None => out.push(b'%'),
            },
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `GET /callback?code=x&state=y HTTP/1.1` -> path and query parameters.
fn parse_request_line(line: &str) -> (String, Vec<(String, String)>) {
    let target = line.split_whitespace().nth(1).unwrap_or("");
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let params = query
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (percent_decode(k), percent_decode(v))
        })
        .collect();
    (path.to_string(), params)
}

/// The loopback page the browser lands on: moo.moi's look, light or dark. `{ok}` is "ok" or "err";
/// the title and text are HTML-escaped by `page`.
const PAGE: &str = r#"<!doctype html><html lang=en><meta charset=utf-8><meta name=viewport content="width=device-width,initial-scale=1"><title>{title} · Moo</title>
<style>
:root{color-scheme:light dark;--fg:#1d1d1f;--dim:#6e6e73;--bg:#f5f5f7;--card:#ffffffcc;--line:#0000001a;--ok:#1f9d55;--err:#d93a3a}
@media (prefers-color-scheme:dark){:root{--fg:#f5f5f7;--dim:#a1a1a6;--bg:#111113;--card:#1c1c1fcc;--line:#ffffff1f;--ok:#34c759;--err:#ff6961}}
*{box-sizing:border-box}
body{margin:0;min-height:100vh;display:flex;align-items:center;justify-content:center;padding:16px;background:radial-gradient(1200px 600px at 50% -10%,#fe588a2e,transparent),var(--bg);color:var(--fg);font:16px/1.5 -apple-system,BlinkMacSystemFont,"SF Pro Text","Helvetica Neue",sans-serif}
main{width:100%;max-width:420px;padding:40px 32px;border:1px solid var(--line);border-radius:20px;background:var(--card);backdrop-filter:blur(20px);text-align:center}
.badge{width:56px;height:56px;margin:0 auto 20px;border-radius:50%;display:flex;align-items:center;justify-content:center;color:#fff}
.ok .badge{background:var(--ok)}.err .badge{background:var(--err)}
h1{margin:0 0 8px;font-size:22px;font-weight:600;letter-spacing:-.01em}
p{margin:0;color:var(--dim);overflow-wrap:anywhere}
.hint{margin-top:24px;font-size:14px}
kbd{font:13px ui-monospace,Menlo,monospace;padding:2px 6px;border:1px solid var(--line);border-radius:6px}
.err .hint{display:none}
</style>
<main class="{ok}">
<div class=badge aria-hidden=true><svg width=28 height=28 viewBox="0 0 24 24" fill=none stroke=currentColor stroke-width=3 stroke-linecap=round stroke-linejoin=round>{mark}</svg></div>
<h1>{title}</h1>
<p>{text}</p>
<p class=hint>You can close this tab and go back to Moo.</p>
</main>
<script>history.replaceState(null,"","/callback")</script>
"#;

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn page(ok: bool, title: &str, text: &str) -> String {
    let mark = if ok { r#"<path d="M5 12.5l4.5 4.5L19 7.5"/>"# } else { r#"<path d="M7 7l10 10M17 7L7 17"/>"# };
    PAGE.replace("{ok}", if ok { "ok" } else { "err" })
        .replace("{mark}", mark)
        .replace("{title}", &escape_html(title))
        .replace("{text}", &escape_html(text))
}

/// Run the browser sign-in. `open(url)` shows the authorize page; waits up to 5 minutes for the
/// redirect unless `cancel` is set. Blocks: run on a worker thread.
pub fn login(ep: &Endpoints, open: impl FnOnce(&str), cancel: &AtomicBool) -> Result<Tokens, String> {
    if ep.client_id.is_empty() {
        return Err("no OAuth client id configured".into());
    }
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| format!("cannot listen for the sign-in redirect: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let (verifier, challenge) = pkce()?;
    let (redirect, state) = if ep.relay.is_empty() {
        (format!("http://127.0.0.1:{port}/callback"), base64url(&random(16)?))
    } else {
        (ep.relay.clone(), format!("{port}.{}", base64url(&random(16)?)))
    };
    let enc = crate::http::percent_encode;
    let mut url = format!(
        "{}?response_type=code&client_id={}&redirect_uri={}&code_challenge={}&code_challenge_method=S256&state={}",
        ep.authorize,
        enc(&ep.client_id),
        enc(&redirect),
        challenge,
        state
    );
    if !ep.scope.is_empty() {
        url.push_str(&format!("&scope={}", enc(&ep.scope)));
    }
    // Extra parameters (a plugin's `params`) can't repeat the ones that carry this sign-in's
    // security: a second redirect_uri, state or challenge would be ambiguous to the server.
    const RESERVED: [&str; 7] = ["response_type", "client_id", "redirect_uri", "state", "code_challenge", "code_challenge_method", "scope"];
    for (k, v) in &ep.extra {
        if RESERVED.contains(&k.to_ascii_lowercase().as_str()) {
            continue;
        }
        url.push_str(&format!("&{}={}", enc(k), enc(v)));
    }
    open(&url);
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(300);
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("sign-in cancelled".into());
        }
        if Instant::now() > deadline {
            return Err("sign-in timed out".into());
        }
        let stream = match listener.accept() {
            Ok((s, _)) => s,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }
            Err(e) => return Err(e.to_string()),
        };
        let _ = stream.set_nonblocking(false);
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        let mut line = String::new();
        if BufReader::new(stream.try_clone().map_err(|e| e.to_string())?).read_line(&mut line).is_err() {
            continue;
        }
        let (path, params) = parse_request_line(&line);
        let get = |k: &str| params.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone()).unwrap_or_default();
        let reply = |title: &str, text: &str| {
            let body = page(title != "Sign-in failed", title, text);
            let mut s = &stream;
            let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        };
        if path != "/callback" {
            let mut s = &stream;
            let _ = write!(s, "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            continue;
        }
        // Only a callback carrying this sign-in's state may end it, with a code or an error. Anything
        // else (another page, a stale tab, a forged ?error=) is answered and ignored, so it can't
        // abort the sign-in.
        if get("state") != state {
            reply("Sign-in failed", "The response did not match this sign-in. Try again from Moo.");
            continue;
        }
        if !get("error").is_empty() {
            let why = if get("error_description").is_empty() { get("error") } else { get("error_description") };
            reply("Sign-in failed", &why);
            return Err(why);
        }
        let code = get("code");
        if code.is_empty() {
            reply("Sign-in failed", "The response had no authorization code. Try again from Moo.");
            return Err("no authorization code in the response".into());
        }
        let result = token_request(
            ep,
            &[("grant_type", "authorization_code"), ("code", &code), ("redirect_uri", &redirect), ("client_id", &ep.client_id), ("code_verifier", &verifier)],
            "",
        );
        match &result {
            Ok(_) => reply("You're signed in", "Moo is connected to your account."),
            Err(e) => reply("Sign-in failed", e),
        }
        return result;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn pkce_matches_rfc_7636() {
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(base64url(&sha256(verifier.as_bytes())), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
        let (v, c) = pkce().unwrap();
        assert_eq!(v.len(), 43);
        assert_eq!(c, base64url(&sha256(v.as_bytes())));
        assert_eq!(base64url(b"f"), "Zg");
        assert_eq!(base64url(b"fo"), "Zm8");
        assert_eq!(base64url(b"foo"), "Zm9v");
    }

    #[test]
    fn request_lines_and_tokens() {
        let (path, p) = parse_request_line("GET /callback?code=a%2Fb+c&state=xyz HTTP/1.1");
        assert_eq!(path, "/callback");
        assert_eq!(p, vec![("code".into(), "a/b c".into()), ("state".into(), "xyz".into())]);
        let t = parse_tokens(r#"{"access_token":"at","refresh_token":"rt","expires_in":3600}"#, "").unwrap();
        assert_eq!((t.access.as_str(), t.refresh.as_str()), ("at", "rt"));
        assert!(t.expires_at > 0.0);
        let kept = parse_tokens(r#"{"access_token":"at2"}"#, "old").unwrap();
        assert_eq!(kept.refresh, "old", "a refresh without a new refresh token keeps the old one");
        assert!(parse_tokens(r#"{"error":"invalid_grant","error_description":"code expired"}"#, "").unwrap_err().contains("code expired"));
    }

    static VERIFIER_SEEN: Mutex<String> = Mutex::new(String::new());

    /// The whole flow against a local token endpoint, with the "browser" played by a thread that
    /// follows the redirect.
    #[test]
    fn login_through_the_loopback_redirect() {
        let token_base = crate::http::tests::serve(|line, _h, body| {
            if line.starts_with("POST /token") {
                *VERIFIER_SEEN.lock().unwrap() = body.to_string();
                (200, "application/json", vec![r#"{"access_token":"AT","refresh_token":"RT","expires_in":60}"#.into()])
            } else {
                (404, "text/plain", vec![])
            }
        });
        let ep = Endpoints { authorize: "https://example.invalid/authorize".into(), token: format!("{token_base}/token"), client_id: "moo".into(), scope: String::new(), relay: String::new(), extra: Vec::new() };
        let cancel = AtomicBool::new(false);
        let tokens = login(
            &ep,
            |url| {
                let url = url.to_string();
                std::thread::spawn(move || {
                    let q = url.split_once('?').unwrap().1;
                    let get = |k: &str| q.split('&').find_map(|p| p.strip_prefix(&format!("{k}="))).unwrap().to_string();
                    let redirect = percent_decode(&get("redirect_uri"));
                    assert_eq!(get("code_challenge_method"), "S256");
                    let hostport = redirect.trim_start_matches("http://").split('/').next().unwrap().to_string();
                    let mut s = std::net::TcpStream::connect(&hostport).unwrap();
                    write!(s, "GET /callback?code=C0DE&state={} HTTP/1.1\r\nHost: x\r\n\r\n", get("state")).unwrap();
                    let mut page = String::new();
                    let _ = std::io::Read::read_to_string(&mut s, &mut page);
                    assert!(page.contains("Signed in"), "{page}");
                });
            },
            &cancel,
        )
        .unwrap();
        assert_eq!((tokens.access.as_str(), tokens.refresh.as_str()), ("AT", "RT"));
        let body = VERIFIER_SEEN.lock().unwrap().clone();
        assert!(body.contains("grant_type=authorization_code") && body.contains("code=C0DE") && body.contains("code_verifier="), "{body}");
    }

    static RELAY_TOKEN_BODY: Mutex<String> = Mutex::new(String::new());

    /// With a relay the authorize request names the relay page and `state` carries the listener's
    /// port; the "relay" here forwards to that port as moo.moi does.
    #[test]
    fn login_through_a_relay_page() {
        let token_base = crate::http::tests::serve(|line, _h, body| {
            if line.starts_with("POST /token") {
                *RELAY_TOKEN_BODY.lock().unwrap() = body.to_string();
                (200, "application/json", vec![r#"{"access_token":"AT2","expires_in":60}"#.into()])
            } else {
                (404, "text/plain", vec![])
            }
        });
        let relay = "https://moo.example/callback";
        let ep = Endpoints { authorize: "https://example.invalid/authorize".into(), token: format!("{token_base}/token"), client_id: "moo".into(), scope: String::new(), relay: relay.into(), extra: Vec::new() };
        let tokens = login(
            &ep,
            |url| {
                let url = url.to_string();
                std::thread::spawn(move || {
                    let q = url.split_once('?').unwrap().1;
                    let get = |k: &str| q.split('&').find_map(|p| p.strip_prefix(&format!("{k}="))).unwrap().to_string();
                    assert_eq!(percent_decode(&get("redirect_uri")), "https://moo.example/callback");
                    let state = get("state");
                    let port: u16 = state.split_once('.').unwrap().0.parse().unwrap();
                    let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
                    write!(s, "GET /callback?code=RELAYED&state={state} HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
                    let mut page = String::new();
                    let _ = std::io::Read::read_to_string(&mut s, &mut page);
                    assert!(page.contains("Signed in"), "{page}");
                });
            },
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(tokens.access, "AT2");
        let body = RELAY_TOKEN_BODY.lock().unwrap().clone();
        assert!(body.contains("code=RELAYED") && body.contains("redirect_uri=https%3A%2F%2Fmoo.example%2Fcallback"), "{body}");
    }

    /// Against a running web/ server: `MOO_TEST_RELAY=http://127.0.0.1:<port>/callback cargo test
    /// --lib -- --ignored moo_web_relay`. curl plays the browser landing on the relay.
    #[test]
    #[ignore]
    fn moo_web_relay() {
        let relay = std::env::var("MOO_TEST_RELAY").expect("MOO_TEST_RELAY");
        let token_base = crate::http::tests::serve(|line, _h, _b| {
            if line.starts_with("POST /token") {
                (200, "application/json", vec![r#"{"access_token":"WEB","expires_in":60}"#.into()])
            } else {
                (404, "text/plain", vec![])
            }
        });
        let ep = Endpoints { authorize: "https://example.invalid/authorize".into(), token: format!("{token_base}/token"), client_id: "moo".into(), scope: String::new(), relay: relay.clone(), extra: Vec::new() };
        let tokens = login(
            &ep,
            |url| {
                let state = url.split("state=").nth(1).unwrap().split('&').next().unwrap().to_string();
                let landing = format!("{relay}?code=FROMWEB&state={state}");
                std::thread::spawn(move || {
                    let out = std::process::Command::new("curl").args(["-s", "-L", "-o", "/dev/null", "-w", "%{http_code} %{url_effective}", &landing]).output().unwrap();
                    let out = String::from_utf8_lossy(&out.stdout).to_string();
                    assert!(out.starts_with("200 http://127.0.0.1:") && out.contains("/callback?code=FROMWEB&state="), "{out}");
                });
            },
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(tokens.access, "WEB");
    }
}
