//! Shell shortcuts: `/bin/sh -c` on a worker thread, results delivered to Tish on the main queue.
//! Output is capped and a command that outlives the timeout is killed, so a stuck script cannot
//! pile up threads or memory.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use dispatch2::DispatchQueue;
use tishlang_core::Value;
const MAX_OUTPUT: usize = 1 << 20;
const TIMEOUT: Duration = Duration::from_secs(60);

static NEXT: AtomicU64 = AtomicU64::new(1);

thread_local! {
    static CALLBACKS: RefCell<HashMap<u64, Value>> = RefCell::new(HashMap::new());
}

pub struct Output {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
    pub ms: f64,
    pub timed_out: bool,
}

fn read_capped(mut r: impl Read + Send + 'static) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = (&mut r).take(MAX_OUTPUT as u64).read_to_end(&mut buf);
        // Drain the rest so the child never blocks on a full pipe.
        let _ = std::io::copy(&mut r, &mut std::io::sink());
        String::from_utf8_lossy(&buf).into_owned()
    })
}

/// Apps started from Finder get a minimal PATH; add the usual tool locations.
fn path_env() -> String {
    let base = std::env::var("PATH").unwrap_or_default();
    let mut parts: Vec<&str> = base.split(':').filter(|p| !p.is_empty()).collect();
    for extra in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin", "/usr/sbin", "/sbin"] {
        if !parts.contains(&extra) {
            parts.push(extra);
        }
    }
    parts.join(":")
}

extern "C" {
    #[link_name = "killpg"]
    fn libc_killpg(pgrp: i32, sig: i32) -> i32;
}

/// Run `cmd` and wait (call off the main thread). `args` are the script's `$1`, `$2`, ...
/// (values are never spliced into `cmd`).
pub fn run_blocking(cmd: &str, cwd: &str, args: &[String]) -> Output {
    run_with_timeout(cmd, cwd, args, TIMEOUT)
}

fn run_with_timeout(cmd: &str, cwd: &str, args: &[String], timeout: Duration) -> Output {
    let t0 = Instant::now();
    let mut command = Command::new("/bin/sh");
    command.arg("-c").arg(cmd).arg("moo").args(args).process_group(0).env("PATH", path_env()).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    if !cwd.is_empty() && std::path::Path::new(cwd).is_dir() {
        command.current_dir(cwd);
    } else if let Some(home) = std::env::var_os("HOME") {
        command.current_dir(home);
    }
    let mut child = match command.spawn() {
        Ok(c) => c,
        Err(e) => {
            return Output { code: 127, stdout: String::new(), stderr: format!("cannot run /bin/sh: {e}"), ms: 0.0, timed_out: false }
        }
    };
    let out = read_capped(child.stdout.take().expect("piped"));
    let err = read_capped(child.stderr.take().expect("piped"));
    let mut timed_out = false;
    let code = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code().unwrap_or(-1),
            Ok(None) if t0.elapsed() > timeout => {
                // The shell is its own process group: kill the whole group, so a pipeline or a
                // backgrounded child can't keep stdout open and leave the reader threads waiting.
                unsafe { libc_killpg(child.id() as i32, 9) };
                let _ = child.kill();
                let _ = child.wait();
                timed_out = true;
                break -1;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
            Err(_) => break -1,
        }
    };
    let mut stderr = err.join().unwrap_or_default();
    if timed_out {
        stderr.push_str(&format!("killed after {} s\n", timeout.as_secs()));
    }
    Output { code, stdout: out.join().unwrap_or_default(), stderr, ms: t0.elapsed().as_secs_f64() * 1000.0, timed_out }
}

/// Run `cmd` in the background; `cb({ id, code, stdout, stderr, ms, timedOut })` on the main thread.
pub fn run(cmd: &str, cwd: &str, args: Vec<String>, cb: Value, to_value: fn(u64, Output) -> Value) -> u64 {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    CALLBACKS.with(|c| c.borrow_mut().insert(id, cb));
    let (cmd, cwd) = (cmd.to_string(), cwd.to_string());
    std::thread::spawn(move || {
        let out = run_blocking(&cmd, &cwd, &args);
        DispatchQueue::main().exec_async(move || {
            let Some(Value::Function(f)) = CALLBACKS.with(|c| c.borrow_mut().remove(&id)) else { return };
            crate::mac::with_ui(|| {
                let _ = f.call(&[to_value(id, out)]);
            });
        });
    });
    id
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_output_status_and_quoting() {
        let o = run_blocking("printf '%s|' \"$PWD\" 'a b'; echo oops >&2; exit 3", "/tmp", &[]);
        assert_eq!(o.code, 3);
        assert!(o.stdout.ends_with("|a b|"), "{}", o.stdout);
        assert_eq!(o.stderr, "oops\n");
        // Values arrive as arguments; shell syntax in them is never run.
        let evil = vec!["it's; $(touch /tmp/moo-pwned) `id`".to_string()];
        assert_eq!(run_blocking("printf %s \"$1\"", "", &evil).stdout, evil[0]);
        assert_eq!(run_blocking("printf %s '\"$1\"'", "", &evil).stdout, "\"$1\"", "a quoted placeholder stays literal");
    }

    /// A timed-out pipeline used to leave `sleep` holding stdout open, so the reader thread (and
    /// the whole call) never returned.
    #[test]
    fn timeout_kills_the_whole_pipeline() {
        let t0 = Instant::now();
        let o = run_with_timeout("sleep 30 | cat; echo done", "", &[], Duration::from_secs(1));
        assert!(o.timed_out);
        assert!(t0.elapsed() < Duration::from_secs(10), "took {:?}", t0.elapsed());
    }

    #[test]
    fn caps_large_output() {
        let o = run_blocking("yes 0123456789 | head -c 3000000", "", &[]);
        assert_eq!(o.code, 0);
        assert_eq!(o.stdout.len(), MAX_OUTPUT);
    }
}
