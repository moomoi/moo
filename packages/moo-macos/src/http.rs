//! Blocking HTTP for the plugin host and OAuth token requests (reqwest, which the app build already
//! links for `tish:http`). Never call on the main thread. Redirects are followed only when the
//! request allows it: plugins' allow-lists and OAuth token endpoints are checked on the URL asked
//! for, so neither may be redirected elsewhere.

use std::sync::OnceLock;
use std::time::Duration;

pub struct Request<'a> {
    pub method: &'a str,
    pub url: &'a str,
    pub headers: Vec<(&'a str, String)>,
    pub body: Vec<u8>,
    /// Longest wait for the whole response, in seconds.
    pub timeout: f64,
    /// Follow 3xx redirects (plugins can't: their allow-list is checked on the URL they ask for).
    pub follow_redirects: bool,
}

#[cfg(test)]
impl<'a> Request<'a> {
    pub fn get(url: &'a str) -> Self {
        Request { method: "GET", url, headers: Vec::new(), body: Vec::new(), timeout: 20.0, follow_redirects: true }
    }

    pub fn header(mut self, name: &'a str, value: impl Into<String>) -> Self {
        self.headers.push((name, value.into()));
        self
    }
}

/// One client per redirect policy, so connections are reused.
fn client(follow_redirects: bool) -> Result<&'static reqwest::blocking::Client, String> {
    static FOLLOW: OnceLock<Result<reqwest::blocking::Client, String>> = OnceLock::new();
    static STAY: OnceLock<Result<reqwest::blocking::Client, String>> = OnceLock::new();
    let (cell, policy) = if follow_redirects {
        (&FOLLOW, reqwest::redirect::Policy::limited(5))
    } else {
        (&STAY, reqwest::redirect::Policy::none())
    };
    cell.get_or_init(|| {
        reqwest::blocking::Client::builder()
            .redirect(policy)
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| e.to_string())
    })
    .as_ref()
    .map_err(|e| e.clone())
}

/// The status and the whole body as text. Blocks: never call on the main thread.
pub fn fetch(req: &Request) -> Result<(u16, String), String> {
    let method = reqwest::Method::from_bytes(req.method.as_bytes()).map_err(|_| format!("bad method {}", req.method))?;
    let url = reqwest::Url::parse(req.url).map_err(|_| format!("invalid URL {}", req.url))?;
    let mut b = client(req.follow_redirects)?.request(method, url).timeout(Duration::from_secs_f64(req.timeout.max(1.0)));
    for (k, v) in &req.headers {
        b = b.header(*k, v.as_str());
    }
    if !req.body.is_empty() {
        b = b.body(req.body.clone());
    }
    let res = b.send().map_err(|e| e.without_url().to_string())?;
    let status = res.status().as_u16();
    let text = res.text().map_err(|e| e.without_url().to_string())?;
    Ok((status, text))
}

/// `application/x-www-form-urlencoded` body.
pub fn form(pairs: &[(&str, &str)]) -> Vec<u8> {
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", percent_encode(k), percent_encode(v)))
        .collect::<Vec<_>>()
        .join("&")
        .into_bytes()
}

/// Percent-encode everything but `A-Z a-z 0-9 - _ . ~`.
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

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;

    /// A one-request-per-connection HTTP server on 127.0.0.1 for tests. `respond(request_line,
    /// headers, body) -> (status, content type, body chunks)`; chunks are written with a short
    /// pause between them, like a streaming server.
    pub fn serve(respond: fn(&str, &str, &str) -> (u16, &'static str, Vec<String>)) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                std::thread::spawn(move || {
                    let mut r = BufReader::new(stream.try_clone().unwrap());
                    let mut line = String::new();
                    r.read_line(&mut line).unwrap();
                    let mut headers = String::new();
                    let mut len = 0usize;
                    loop {
                        let mut h = String::new();
                        r.read_line(&mut h).unwrap();
                        if h.trim().is_empty() {
                            break;
                        }
                        if let Some(v) = h.to_lowercase().strip_prefix("content-length:") {
                            len = v.trim().parse().unwrap_or(0);
                        }
                        headers.push_str(&h);
                    }
                    let mut body = vec![0u8; len];
                    r.read_exact(&mut body).unwrap();
                    let (status, ctype, chunks) = respond(line.trim(), &headers, &String::from_utf8_lossy(&body));
                    let mut s = stream;
                    let _ = write!(s, "HTTP/1.1 {status} X\r\nContent-Type: {ctype}\r\nConnection: close\r\n\r\n");
                    for c in chunks {
                        let _ = s.write_all(c.as_bytes());
                        let _ = s.flush();
                        std::thread::sleep(Duration::from_millis(20));
                    }
                });
            }
        });
        format!("http://127.0.0.1:{}", addr.port())
    }

    #[test]
    fn fetches_and_does_not_follow_redirects_when_told() {
        let base = serve(|line, headers, body| {
            if line.starts_with("POST /echo") {
                let auth = headers.lines().find(|h| h.to_lowercase().starts_with("authorization")).unwrap_or("").trim().to_string();
                (200, "text/plain", vec![format!("{auth}\n{body}")])
            } else {
                (404, "text/plain", vec!["nope".into()])
            }
        });
        let url = format!("{base}/echo");
        let mut post = Request::get(&url).header("Authorization", "Bearer k");
        post.method = "POST";
        post.body = b"{\"a\":1}".to_vec();
        let (status, body) = fetch(&post).unwrap();
        assert_eq!(status, 200);
        assert_eq!(body.to_lowercase(), "authorization: bearer k\n{\"a\":1}");

        let missing = format!("{base}/missing");
        assert_eq!(fetch(&Request::get(&missing)).unwrap(), (404, "nope".into()));

        assert!(fetch(&Request::get("http://127.0.0.1:1/")).is_err(), "connection refused is an error");

        // A plugin's or token request's redirect is answered, not followed.
        // (The content type smuggles in a Location header pointing at a closed port.)
        let moved = serve(|_l, _h, _b| (302, "text/plain\r\nLocation: http://127.0.0.1:1/elsewhere", vec![]));
        let target = format!("{moved}/x");
        let mut stay = Request::get(&target);
        stay.follow_redirects = false;
        assert_eq!(fetch(&stay).unwrap().0, 302);
        assert!(fetch(&Request::get(&target)).is_err(), "followed to the closed port");
    }
}
