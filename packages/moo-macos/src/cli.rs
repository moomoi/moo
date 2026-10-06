//! The `moo` command line. The app binary doubles as its own client: run with arguments, it
//! connects to the running app's Unix socket, sends them, prints what comes back and exits with
//! the app's exit code. Nothing else runs in the client (no window, no index), so hotkey daemons
//! (skhd, Karabiner, BetterTouchTool, Keyboard Maestro, Hammerspoon) and scripts can call it cheaply.
//!
//! Protocol, one JSON object per line. Client: `{"args": [...], "cwd": "..."}`. App, any number of
//! `{"out": "..."}` / `{"err": "..."}` then `{"exit": n}`. Requests are handled on the main thread
//! by the Tish `onCli(args, cwd, token)` callback, which answers with `cliWrite` / `cliEnd`, now or
//! later (an AI answer streams).

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tishlang_core::{json_parse, Value};

pub const USAGE: &str = "\
usage: moo [command] [args]

  (no command)                 show the launcher (starts Moo if needed)
  toggle | show | hide | quit
  search <text>                show the launcher with <text> typed
  category <name> [text]       show Applications, Files, Actions, Clipboard or Recent searches
  key <name>                   act as if a key was pressed in the open launcher (tab, shift+tab, enter, escape, …)
  type <text>                  insert text into the focused field, one character every 120 ms
  history [--clear]            recent searches, newest first
  run <keyword|id> [text]      run a shortcut or command (ids: moo list commands)
  open <path|url>              open with the default app
  files <query> [-n N] [--contents]
                               file paths, best match first; --contents searches inside files
  trash <path>                 move to the Trash; prints where it went
  open-with <path> [app]       list the apps that open it (* default), or open it in one
  apps <query> [-n N]          matching applications
  ask [--model M] <question>   ask Ask AI's model (or M); the answer streams, tools included
  ai                           the model Ask AI uses and each provider's sign-in
  ai models [text] [--json]    every model (* current): Apple's, Hypery's (Claude, Grok, Gemini,
                               GPT, …), OpenAI's, local Ollama and LM Studio ones
  ai use <model>               make Ask AI use it: apple, hypery:<id>, ollama:<id>, … or default
  ai login | logout [provider] sign in to Hypery in the browser, or forget its key and sign-in
  ai key [provider] <key>      keep an API key in the Keychain (Hypery when no provider)
  ai usage [provider]          Hypery balance and this month's spending
  ai team [name]               Hypery's teams (* current), or switch to one
  ai client-id <id> [provider] the OAuth app id browser sign-in uses
  calc <expression>            the plain answer: 2^10, 5 km in mi, 100 usd to eur, 255 in hex,
                               time in tokyo, 3pm pst to cet (--json adds display and detail)
  web <text>                   search suggestions from the default engine
  web engine [name]            list engines (* default) or pick one: google, duckduckgo, bing,
                               brave
  define <word>                the word's senses, pronunciation and origin from the macOS
                               dictionary, every homograph (--json for the parsed entry)
  contacts [name]              contacts whose name matches (all when empty); needs access,
                               which Search Contacts in Moo asks for
  system [command] [arg]       list system commands, or run one: lock, sleep, sleep-displays,
                               screen-saver, dark-mode, mute, volume, eject, hide-all, quit-all,
                               empty-trash, restart, shut-down, log-out
  window [layout]              list layouts, or arrange the frontmost window: left-half,
                               right-third, maximize, center, next-display, restore, …
  selection                    the selected text in the frontmost app
  running [--json]             apps with a Dock icon, most memory first
  running <action> <name|pid>  switch, hide, unhide, quit or force-quit one of them
  volume [N|up|down]           print or set the output volume
  mute [on|off]                toggle or set mute
  dark-mode [on|off]           toggle or set dark mode
  clipboard [-n N]             clipboard history, newest first
  list [shortcuts|commands|hotkeys]
  shortcut add <keyword> <kind> <target> [--name N] [--hotkey K] [--output show|copy|none] [--input T] [--expand]
                               kinds: url, open, command, shell, text; --expand makes a text
                               shortcut a snippet, replaced wherever its keyword is typed
  shortcut rm <keyword>
  hotkey add <keys> <keyword|id|app> [text]
                               bind a shortcut, a command id or an app's path
                               (/Applications/Safari.app)
  hotkey rm <keys>
  config                       print the path of shortcuts.json
  status
  --version                    this binary's version and path

  --json                       machine-readable output (files, apps, list, clipboard, status)

Templates take {query}, {clipboard}, {date} and {time}.
";

/// Release builds set `MOO_VERSION` (scripts/build-universal.sh); local builds say "dev".
pub const VERSION: &str = match option_env!("MOO_VERSION") {
    Some(v) => v,
    None => "dev",
};

const MAX_REQUEST: u64 = 1 << 20;

pub fn socket_path() -> PathBuf {
    if let Some(p) = std::env::var_os("MOO_SOCKET") {
        return PathBuf::from(p);
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    home.join("Library/Application Support/Moo/moo.sock")
}

/// Arguments meant for Moo: launch-services and Xcode extras (`-psn_…`, `-NSDocument… YES`)
/// are dropped.
pub fn args() -> Vec<String> {
    let mut out = Vec::new();
    let mut it = std::env::args().skip(1).peekable();
    while let Some(a) = it.next() {
        if a.starts_with("-psn_") {
            continue;
        }
        if a.starts_with("-NS") || a.starts_with("-Apple") {
            it.next();
            continue;
        }
        out.push(a);
    }
    out
}

pub fn running() -> bool {
    UnixStream::connect(socket_path()).is_ok()
}

fn json_line(key: &str, value: &str) -> String {
    let mut s = String::with_capacity(value.len() + 16);
    s.push_str("{\"");
    s.push_str(key);
    s.push_str("\":\"");
    tishlang_core::escape_json_string_into(&mut s, value);
    s.push_str("\"}\n");
    s
}

fn request_json(args: &[String], cwd: &str) -> String {
    let mut s = String::from("{\"args\":[");
    for (i, a) in args.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push('"');
        tishlang_core::escape_json_string_into(&mut s, a);
        s.push('"');
    }
    s.push_str("],\"cwd\":\"");
    tishlang_core::escape_json_string_into(&mut s, cwd);
    s.push_str("\"}\n");
    s
}

// ── Client ──────────────────────────────────────────────────────────────────

/// Start the app in the background and wait for its socket.
fn start_app() -> Result<UnixStream, String> {
    let exe = std::env::current_exe().and_then(|p| p.canonicalize()).map_err(|e| format!("cannot find the Moo binary: {e}"))?;
    let mut cmd = std::process::Command::new(&exe);
    cmd.env("MOO_START_HIDDEN", "1")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    if let Some(dir) = exe.parent() {
        cmd.current_dir(dir);
    }
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    cmd.spawn().map_err(|e| format!("cannot start Moo: {e}"))?;
    let t0 = Instant::now();
    loop {
        if let Ok(s) = UnixStream::connect(socket_path()) {
            return Ok(s);
        }
        if t0.elapsed() > Duration::from_secs(10) {
            return Err("Moo did not start within 10 s".into());
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn str_field(v: &Value, key: &str) -> Option<String> {
    match v {
        Value::Object(o) => match o.borrow().strings.get(key) {
            Some(Value::String(s)) => Some(s.to_string()),
            _ => None,
        },
        _ => None,
    }
}

fn num_field(v: &Value, key: &str) -> Option<f64> {
    match v {
        Value::Object(o) => match o.borrow().strings.get(key) {
            Some(Value::Number(n)) => Some(*n),
            _ => None,
        },
        _ => None,
    }
}

/// Send `args` to the running app (starting it if needed), print the reply; the exit code.
pub fn client(args: &[String]) -> i32 {
    let first = args.first().map(String::as_str).unwrap_or("");
    if matches!(first, "help" | "-h" | "--help") {
        print!("{USAGE}");
        return 0;
    }
    if matches!(first, "-v" | "--version") {
        let exe = std::env::current_exe().ok().and_then(|p| p.canonicalize().ok());
        match exe {
            Some(p) => println!("Moo {VERSION} ({})", p.display()),
            None => println!("Moo {VERSION}"),
        }
        return 0;
    }
    let stream = match UnixStream::connect(socket_path()) {
        Ok(s) => s,
        Err(_) if first == "quit" || first == "hide" => return 0,
        Err(_) if first == "status" => {
            println!("Moo is not running");
            return 1;
        }
        Err(_) => match start_app() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("moo: {e}");
                return 1;
            }
        },
    };
    let cwd = std::env::current_dir().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    let mut writer = match stream.try_clone() {
        Ok(w) => w,
        Err(e) => {
            eprintln!("moo: {e}");
            return 1;
        }
    };
    if let Err(e) = writer.write_all(request_json(args, &cwd).as_bytes()) {
        eprintln!("moo: cannot talk to Moo: {e}");
        return 1;
    }
    let mut stdout = std::io::stdout().lock();
    let mut stderr = std::io::stderr().lock();
    for line in BufReader::new(stream).lines() {
        let Ok(line) = line else { break };
        let Ok(msg) = json_parse(&line) else { continue };
        if let Some(s) = str_field(&msg, "out") {
            let _ = stdout.write_all(s.as_bytes());
            let _ = stdout.flush();
        } else if let Some(s) = str_field(&msg, "err") {
            let _ = stderr.write_all(s.as_bytes());
        } else if let Some(code) = num_field(&msg, "exit") {
            return code as i32;
        }
    }
    eprintln!("moo: Moo closed the connection");
    1
}

// ── Server ──────────────────────────────────────────────────────────────────

static CONNECTIONS: Mutex<Option<HashMap<u64, UnixStream>>> = Mutex::new(None);
static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);
static SERVING: Mutex<Option<PathBuf>> = Mutex::new(None);

thread_local! {
    static HANDLER: RefCell<Option<Value>> = const { RefCell::new(None) };
}

pub struct Request {
    pub token: u64,
    pub args: Vec<String>,
    pub cwd: String,
}

fn read_request(stream: &UnixStream) -> Option<(Vec<String>, String)> {
    let mut line = String::new();
    BufReader::new(stream.take(MAX_REQUEST)).read_line(&mut line).ok()?;
    let v = json_parse(&line).ok()?;
    let args = match &v {
        Value::Object(o) => match o.borrow().strings.get("args") {
            Some(Value::Array(a)) => a
                .borrow()
                .iter()
                .map(|x| match x {
                    Value::String(s) => s.to_string(),
                    Value::Number(n) => n.to_string(),
                    _ => String::new(),
                })
                .collect(),
            _ => Vec::new(),
        },
        _ => return None,
    };
    Some((args, str_field(&v, "cwd").unwrap_or_default()))
}

/// Listen on `socket_path()`. Each request is handed to `dispatch` (which must get it to the main
/// thread) with a token for `write` / `end`. Fails when another instance is already listening.
pub fn serve(dispatch: impl Fn(Request) + Send + Sync + 'static) -> Result<PathBuf, String> {
    let path = socket_path();
    if UnixStream::connect(&path).is_ok() {
        return Err(format!("another Moo is listening on {}", path.display()));
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    CONNECTIONS.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new);
    let dispatch = std::sync::Arc::new(dispatch);
    std::thread::Builder::new()
        .name("moo-cli".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let dispatch = dispatch.clone();
                std::thread::spawn(move || {
                    let Some((args, cwd)) = read_request(&stream) else { return };
                    let token = NEXT_TOKEN.fetch_add(1, Ordering::Relaxed);
                    if let Some(m) = CONNECTIONS.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
                        m.insert(token, stream);
                    }
                    dispatch(Request { token, args, cwd });
                });
            }
        })
        .map_err(|e| e.to_string())?;
    *SERVING.lock().unwrap_or_else(|e| e.into_inner()) = Some(path.clone());
    Ok(path)
}

/// Remove the socket this process serves, so clients see "not running" right away on quit.
pub fn stop_serving() {
    if let Some(path) = SERVING.lock().unwrap_or_else(|e| e.into_inner()).take() {
        let _ = std::fs::remove_file(path);
    }
}

/// Send output to the client of `token`: `stream` is "out" or "err". False once it has gone.
pub fn write(token: u64, stream: &str, text: &str) -> bool {
    let mut guard = CONNECTIONS.lock().unwrap_or_else(|e| e.into_inner());
    let Some(conn) = guard.as_mut().and_then(|m| m.get_mut(&token)) else { return false };
    let key = if stream == "err" { "err" } else { "out" };
    if conn.write_all(json_line(key, text).as_bytes()).is_err() {
        guard.as_mut().map(|m| m.remove(&token));
        return false;
    }
    true
}

/// Finish the request with an exit code and close the connection.
pub fn end(token: u64, code: i32) -> bool {
    let conn = CONNECTIONS.lock().unwrap_or_else(|e| e.into_inner()).as_mut().and_then(|m| m.remove(&token));
    match conn {
        Some(mut c) => c.write_all(format!("{{\"exit\":{code}}}\n").as_bytes()).is_ok(),
        None => false,
    }
}

/// The Tish handler, called on the main thread.
pub fn set_handler(f: Option<Value>) {
    HANDLER.with(|h| *h.borrow_mut() = f);
}

pub fn handler() -> Option<Value> {
    HANDLER.with(|h| h.borrow().clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_and_reply_lines_are_valid_json() {
        let req = request_json(&["run".into(), "g".into(), "a \"b\"\n".into()], "/tmp/x y");
        let v = json_parse(&req).unwrap();
        assert_eq!(str_field(&v, "cwd").as_deref(), Some("/tmp/x y"));
        let out = json_parse(&json_line("out", "line 1\nquote \" tab\t")).unwrap();
        assert_eq!(str_field(&out, "out").as_deref(), Some("line 1\nquote \" tab\t"));
    }

    /// A real socket: the dispatcher answers on another thread, as the main thread does in the app.
    #[test]
    fn socket_round_trip() {
        let dir = std::env::temp_dir().join(format!("moo-cli-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("MOO_SOCKET", dir.join("s.sock"));
        serve(|req: Request| {
            std::thread::spawn(move || {
                write(req.token, "out", &format!("{} in {}\n", req.args.join(" "), req.cwd));
                write(req.token, "err", "warning\n");
                end(req.token, req.args.len() as i32);
            });
        })
        .unwrap();
        assert!(running());
        assert!(serve(|_| {}).unwrap_err().contains("another Moo"));

        let mut s = UnixStream::connect(socket_path()).unwrap();
        s.write_all(request_json(&["files".into(), "a b".into()], "/w").as_bytes()).unwrap();
        let lines: Vec<String> = BufReader::new(s).lines().map(Result::unwrap).collect();
        assert_eq!(lines, vec![r#"{"out":"files a b in /w\n"}"#, r#"{"err":"warning\n"}"#, r#"{"exit":2}"#]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
