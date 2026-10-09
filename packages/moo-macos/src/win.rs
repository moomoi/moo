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

// ── Snippet keywords typed in other apps ────────────────────────────────────

use std::cell::RefCell;

use tishlang_core::Value;
use windows::Win32::UI::Input::KeyboardAndMouse::*;

/// tish-windows's tag on the keystrokes it types (accessibility.rs, SYNTHETIC_KEY_TAG): "MOO\0".
const SYNTHETIC_KEY_TAG: usize = 0x004F_4F4D;

thread_local! {
    static TYPED: RefCell<crate::snippets::Typed> = RefCell::new(Default::default());
    static ON_SNIPPET: RefCell<Option<Value>> = const { RefCell::new(None) };
    static HOOKS: RefCell<Option<(HHOOK, HHOOK)>> = const { RefCell::new(None) };
}

fn foreground_is_us() -> bool {
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(GetForegroundWindow(), Some(&mut pid));
        pid == std::process::id()
    }
}

/// The characters key `vk` types in the foreground app's keyboard layout, given the modifiers held.
fn chars_for(vk: u32, scan: u32) -> String {
    unsafe {
        let mut state = [0u8; 256];
        for k in [VK_SHIFT, VK_LSHIFT, VK_RSHIFT, VK_CAPITAL] {
            let s = GetAsyncKeyState(k.0 as i32) as u16;
            state[k.0 as usize] = if s & 0x8000 != 0 { 0x80 } else { 0 };
        }
        if GetKeyState(VK_CAPITAL.0 as i32) & 1 != 0 {
            state[VK_CAPITAL.0 as usize] |= 1;
        }
        let layout = GetKeyboardLayout(GetWindowThreadProcessId(GetForegroundWindow(), None));
        let mut buf = [0u16; 8];
        // Flag 0x4: don't change the keyboard state (dead keys stay pending for the app).
        let n = ToUnicodeEx(vk, scan, &state, &mut buf, 0x4, Some(layout));
        if n > 0 { String::from_utf16_lossy(&buf[..n as usize]) } else { String::new() }
    }
}

fn held(vk: VIRTUAL_KEY) -> bool {
    unsafe { GetAsyncKeyState(vk.0 as i32) as u16 & 0x8000 != 0 }
}

unsafe extern "system" fn keyboard_hook(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code >= 0 && (wp.0 as u32 == WM_KEYDOWN || wp.0 as u32 == WM_SYSKEYDOWN) {
        let k = &*(lp.0 as *const KBDLLHOOKSTRUCT);
        // Our own expansions (tagged by tish-windows's SendInput) and typing into Moo itself don't
        // count; other tools' synthesized typing (AutoHotkey, remote desktop) does.
        if k.dwExtraInfo != SYNTHETIC_KEY_TAG && !foreground_is_us() {
            let vk = VIRTUAL_KEY(k.vkCode as u16);
            let hit = TYPED.with(|t| {
                let mut t = t.borrow_mut();
                if held(VK_CONTROL) || held(VK_MENU) || held(VK_LWIN) || held(VK_RWIN) {
                    t.reset();
                    return None;
                }
                match vk {
                    VK_BACK => {
                        t.backspace();
                        None
                    }
                    VK_SHIFT | VK_LSHIFT | VK_RSHIFT | VK_CAPITAL => None,
                    VK_LEFT | VK_RIGHT | VK_UP | VK_DOWN | VK_HOME | VK_END | VK_PRIOR | VK_NEXT | VK_ESCAPE | VK_RETURN | VK_TAB | VK_DELETE => {
                        t.reset();
                        None
                    }
                    _ => {
                        let s = chars_for(k.vkCode, k.scanCode);
                        if s.is_empty() {
                            None
                        } else {
                            t.push(&s)
                        }
                    }
                }
            });
            if let Some(keyword) = hit {
                // Let the app take the keyword's last key before replacing it (as on macOS).
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(40));
                    on_main(move || {
                        if let Some(Value::Function(f)) = ON_SNIPPET.with(|c| c.borrow().clone()) {
                            let _ = f.call(&[Value::String(keyword.as_str().into())]);
                        }
                    });
                });
            }
        }
    }
    CallNextHookEx(None, code, wp, lp)
}

unsafe extern "system" fn mouse_hook(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code >= 0 && matches!(wp.0 as u32, WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN) {
        // A click may move the cursor: what was typed before is no longer before it.
        TYPED.with(|t| t.borrow_mut().reset());
    }
    CallNextHookEx(None, code, wp, lp)
}

/// Watch for `keywords` typed in other apps; `cb(keyword)` on the UI thread. An empty list stops
/// watching. Low-level hooks run on this (the UI) thread's message loop.
pub fn watch_snippets(keywords: Vec<String>, cb: Value) {
    ensure_window();
    TYPED.with(|t| t.borrow_mut().set_keywords(keywords));
    ON_SNIPPET.with(|c| *c.borrow_mut() = Some(cb));
    let watching = !TYPED.with(|t| t.borrow().is_empty());
    HOOKS.with(|h| {
        let mut h = h.borrow_mut();
        match (watching, h.is_some()) {
            (false, true) => {
                if let Some((k, m)) = h.take() {
                    unsafe {
                        let _ = UnhookWindowsHookEx(k);
                        let _ = UnhookWindowsHookEx(m);
                    }
                }
            }
            (true, false) => unsafe {
                let module = GetModuleHandleW(None).ok().map(|m| m.into());
                if let (Ok(k), Ok(m)) = (
                    SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), module, 0),
                    SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), module, 0),
                ) {
                    *h = Some((k, m));
                }
            },
            _ => {}
        }
    });
}
