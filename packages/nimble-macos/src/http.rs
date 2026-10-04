//! HTTP through URLSession (`swift/http.swift`). `start` streams a response line by line to a
//! handler on a Swift thread (server-sent events); `fetch` waits for the whole body and must not be
//! called on the main thread.

use std::collections::HashMap;
use std::ffi::{c_char, CStr, CString};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Mutex};
use std::time::Duration;

type Callback = extern "C" fn(u64, i32, i32, *const c_char);

extern "C" {
    fn nimble_http_start(
        id: u64,
        method: *const c_char,
        url: *const c_char,
        headers: *const c_char,
        body: *const u8,
        body_len: usize,
        timeout: f64,
        cb: Callback,
    ) -> bool;
    fn nimble_http_cancel(id: u64);
}

#[derive(Debug)]
pub enum Event {
    Status(u16),
    Line(String),
    Done,
    Failed(String),
    Cancelled,
}

pub type Handler = Box<dyn FnMut(Event) + Send>;

static HANDLERS: Mutex<Option<HashMap<u64, Handler>>> = Mutex::new(None);
static NEXT: AtomicU64 = AtomicU64::new(1);

pub struct Request<'a> {
    pub method: &'a str,
    pub url: &'a str,
    pub headers: Vec<(&'a str, String)>,
    pub body: Vec<u8>,
    /// Longest wait for more data, in seconds.
    pub timeout: f64,
}

impl<'a> Request<'a> {
    pub fn get(url: &'a str) -> Self {
        Request { method: "GET", url, headers: Vec::new(), body: Vec::new(), timeout: 20.0 }
    }

    pub fn post_json(url: &'a str, body: String) -> Self {
        Request { method: "POST", url, headers: vec![("Content-Type", "application/json".into())], body: body.into_bytes(), timeout: 60.0 }
    }

    pub fn header(mut self, name: &'a str, value: impl Into<String>) -> Self {
        self.headers.push((name, value.into()));
        self
    }
}

fn headers_json(headers: &[(&str, String)]) -> String {
    let mut out = String::from("{");
    for (i, (k, v)) in headers.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push('"');
        tishlang_core::escape_json_string_into(&mut out, k);
        out.push_str("\":\"");
        tishlang_core::escape_json_string_into(&mut out, v);
        out.push('"');
    }
    out.push('}');
    out
}

/// Start `req`; `handler` gets the status, each line, then Done, Failed or Cancelled. None for an
/// invalid URL.
pub fn start(req: &Request, handler: Handler) -> Option<u64> {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    HANDLERS.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(id, handler);
    let c = |s: &str| CString::new(s.replace('\0', "")).unwrap();
    let (method, url, headers) = (c(req.method), c(req.url), c(&headers_json(&req.headers)));
    let ok = unsafe {
        nimble_http_start(id, method.as_ptr(), url.as_ptr(), headers.as_ptr(), req.body.as_ptr(), req.body.len(), req.timeout, on_event)
    };
    if !ok {
        HANDLERS.lock().unwrap_or_else(|e| e.into_inner()).as_mut().map(|m| m.remove(&id));
        return None;
    }
    Some(id)
}

pub fn cancel(id: u64) {
    unsafe { nimble_http_cancel(id) }
}

extern "C" fn on_event(id: u64, kind: i32, status: i32, text: *const c_char) {
    let text = if text.is_null() { String::new() } else { unsafe { CStr::from_ptr(text) }.to_string_lossy().into_owned() };
    let event = match kind {
        0 => Event::Status(status as u16),
        1 => Event::Line(text),
        2 => Event::Done,
        4 => Event::Cancelled,
        _ => Event::Failed(text),
    };
    let last = matches!(event, Event::Done | Event::Failed(_) | Event::Cancelled);
    // Called without the lock held, so a handler may start another request.
    let Some(mut h) = HANDLERS.lock().unwrap_or_else(|e| e.into_inner()).as_mut().and_then(|m| m.remove(&id)) else { return };
    h(event);
    if !last {
        HANDLERS.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(id, h);
    }
}

/// The status and whole body (lines joined with `\n`). Blocks: never call on the main thread.
pub fn fetch(req: &Request) -> Result<(u16, String), String> {
    let (tx, rx) = mpsc::channel();
    let mut status = 0u16;
    let mut body = String::new();
    let handler: Handler = Box::new(move |e| match e {
        Event::Status(s) => status = s,
        Event::Line(l) => {
            if !body.is_empty() {
                body.push('\n');
            }
            body.push_str(&l);
        }
        Event::Done => {
            let _ = tx.send(Ok((status, std::mem::take(&mut body))));
        }
        Event::Failed(m) => {
            let _ = tx.send(Err(m));
        }
        Event::Cancelled => {
            let _ = tx.send(Err("cancelled".into()));
        }
    });
    let id = start(req, handler).ok_or_else(|| format!("invalid URL {}", req.url))?;
    match rx.recv_timeout(Duration::from_secs_f64(req.timeout + 10.0)) {
        Ok(r) => r,
        Err(_) => {
            cancel(id);
            Err(format!("{} timed out", req.url))
        }
    }
}

/// `application/x-www-form-urlencoded` body.
pub fn form(pairs: &[(&str, &str)]) -> Vec<u8> {
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", crate::shortcuts::percent_encode(k), crate::shortcuts::percent_encode(v)))
        .collect::<Vec<_>>()
        .join("&")
        .into_bytes()
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
    fn fetch_and_stream_through_urlsession() {
        let base = serve(|line, headers, body| {
            if line.starts_with("POST /echo") {
                let auth = headers.lines().find(|h| h.to_lowercase().starts_with("authorization")).unwrap_or("").trim().to_string();
                (200, "text/plain", vec![format!("{auth}\n{body}")])
            } else if line.starts_with("GET /lines") {
                (200, "text/event-stream", vec!["data: one\n\n".into(), "data: two\n\n".into(), "data: [DONE]\n\n".into()])
            } else {
                (404, "text/plain", vec!["nope".into()])
            }
        });
        let url = format!("{base}/echo");
        let (status, body) = fetch(&Request::post_json(&url, "{\"a\":1}".into()).header("Authorization", "Bearer k")).unwrap();
        assert_eq!(status, 200);
        assert_eq!(body, "Authorization: Bearer k\n{\"a\":1}");

        let missing = format!("{base}/missing");
        assert_eq!(fetch(&Request::get(&missing)).unwrap(), (404, "nope".into()));

        let (tx, rx) = mpsc::channel();
        let lines = format!("{base}/lines");
        start(&Request::get(&lines), Box::new(move |e| { let _ = tx.send(format!("{e:?}")); })).unwrap();
        let got: Vec<String> = rx.iter().take(5).collect();
        assert_eq!(got, ["Status(200)", "Line(\"data: one\")", "Line(\"data: two\")", "Line(\"data: [DONE]\")", "Done"]);

        assert!(fetch(&Request::get("http://127.0.0.1:1/")).is_err(), "connection refused is an error");
    }
}
