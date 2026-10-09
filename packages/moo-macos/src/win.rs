//! Windows: what the plugin host and the main-thread bridge need from the platform, the
//! counterparts of mac.rs's `launch`, `debug_log` and `with_ui`, plus the hidden window that
//! carries work from worker threads to the UI thread (macOS uses the main dispatch queue).

use std::sync::atomic::{AtomicIsize, Ordering};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::*;

const WM_RUN: u32 = WM_APP + 77;
static WINDOW: AtomicIsize = AtomicIsize::new(0);

type Job = Box<dyn FnOnce() + Send>;

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_RUN {
        let job = Box::from_raw(lp.0 as *mut Job);
        job();
        return LRESULT(0);
    }
    DefWindowProcW(hwnd, msg, wp, lp)
}

/// Make the UI thread's message window. Call on the UI thread before any worker posts.
pub fn ensure_window() {
    if WINDOW.load(Ordering::Relaxed) != 0 {
        return;
    }
    unsafe {
        let Ok(instance) = GetModuleHandleW(None) else { return };
        let class = w!("MooBridge");
        let wc = WNDCLASSW { lpfnWndProc: Some(proc), hInstance: instance.into(), lpszClassName: class, ..Default::default() };
        RegisterClassW(&wc);
        if let Ok(h) = CreateWindowExW(WINDOW_EX_STYLE::default(), class, w!(""), WINDOW_STYLE::default(), 0, 0, 0, 0, Some(HWND_MESSAGE), None, Some(instance.into()), None) {
            WINDOW.store(h.0 as isize, Ordering::Relaxed);
        }
    }
}

/// From any thread: run `f` on the UI thread (dropped if the window is gone).
pub fn on_main(f: impl FnOnce() + Send + 'static) {
    let h = WINDOW.load(Ordering::Relaxed);
    if h == 0 {
        return;
    }
    let job: *mut Job = Box::into_raw(Box::new(Box::new(f)));
    unsafe {
        if PostMessageW(Some(HWND(h as *mut _)), WM_RUN, WPARAM(0), LPARAM(job as isize)).is_err() {
            drop(Box::from_raw(job));
        }
    }
}

/// Open a URL or path with its default handler.
pub fn launch(target: &str) -> bool {
    let t: Vec<u16> = target.encode_utf16().chain(std::iter::once(0)).collect();
    let r = unsafe { ShellExecuteW(None, w!("open"), PCWSTR(t.as_ptr()), None, None, SW_SHOWNORMAL) };
    r.0 as isize > 32
}

pub fn debug_log(msg: &str) {
    if std::env::var_os("MOO_DEBUG").is_some() {
        eprintln!("moo: {msg}");
    }
}

/// Callbacks run as they are on Windows: the host keeps focus itself.
pub fn with_ui<R>(f: impl FnOnce() -> R) -> R {
    f()
}
