//! Windows: what the plugin host and the main-thread bridge need from the platform, the
//! counterparts of mac.rs's `launch`, `debug_log` and `with_ui`, plus the hidden window that
//! carries work from worker threads to the UI thread (macOS uses the main dispatch queue).

use std::sync::atomic::{AtomicIsize, Ordering};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, SIZE, WPARAM};
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

// ── Quick Look: Windows preview handlers ────────────────────────────────────

use windows::core::{Interface, GUID};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Com::{CLSIDFromString, CoCreateInstance, CLSCTX_INPROC_SERVER, CLSCTX_LOCAL_SERVER, STGM_READ};
use windows::Win32::UI::Shell::PropertiesSystem::{IInitializeWithFile, IInitializeWithStream};
use windows::Win32::UI::Shell::{
    AssocQueryStringW, IInitializeWithItem, IPreviewHandler, IShellItem, IShellItemImageFactory, SHCreateItemFromParsingName,
    SHCreateStreamOnFileEx, ASSOCF_INIT_DEFAULTTOSTAR, ASSOCSTR_SHELLEXTENSION, SIIGBF_BIGGERSIZEOK,
};

/// The preview window and what fills it: a preview handler, or a large thumbnail when the file
/// type has none (most images, folders).
struct Preview {
    hwnd: HWND,
    path: String,
    handler: Option<IPreviewHandler>,
    thumbnail: Option<HBITMAP>,
}

thread_local! {
    static PREVIEW: RefCell<Option<Preview>> = const { RefCell::new(None) };
}

/// IID of a preview handler under a file type's `shellex` key.
const PREVIEW_HANDLER_KEY: PCWSTR = w!("{8895b1c6-b41f-4c1c-a562-0d564250836f}");

fn handler_clsid(ext: &str) -> Option<GUID> {
    let e: Vec<u16> = ext.encode_utf16().chain(std::iter::once(0)).collect();
    let mut buf = [0u16; 64];
    let mut n = buf.len() as u32;
    unsafe {
        AssocQueryStringW(ASSOCF_INIT_DEFAULTTOSTAR, ASSOCSTR_SHELLEXTENSION, PCWSTR(e.as_ptr()), PREVIEW_HANDLER_KEY, Some(windows::core::PWSTR(buf.as_mut_ptr())), &mut n).ok().ok()?;
        CLSIDFromString(PCWSTR(buf.as_ptr())).ok()
    }
}

/// A preview handler set up on `path`, through whichever initializer it implements.
fn open_handler(path: &str) -> Option<IPreviewHandler> {
    let ext = std::path::Path::new(path).extension()?.to_string_lossy().to_lowercase();
    let clsid = handler_clsid(&format!(".{ext}"))?;
    let p: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        // Most handlers run in a surrogate process (prevhost.exe); some only in process.
        let h: IPreviewHandler = CoCreateInstance(&clsid, None, CLSCTX_LOCAL_SERVER).or_else(|_| CoCreateInstance(&clsid, None, CLSCTX_INPROC_SERVER)).ok()?;
        if let Ok(i) = h.cast::<IInitializeWithFile>() {
            i.Initialize(PCWSTR(p.as_ptr()), STGM_READ.0).ok()?;
        } else if let Ok(i) = h.cast::<IInitializeWithItem>() {
            let item: IShellItem = SHCreateItemFromParsingName(PCWSTR(p.as_ptr()), None).ok()?;
            i.Initialize(&item, STGM_READ.0).ok()?;
        } else if let Ok(i) = h.cast::<IInitializeWithStream>() {
            let stream = SHCreateStreamOnFileEx(PCWSTR(p.as_ptr()), STGM_READ.0, 0, false, None).ok()?;
            i.Initialize(&stream, STGM_READ.0).ok()?;
        } else {
            return None;
        }
        Some(h)
    }
}

fn thumbnail(path: &str) -> Option<HBITMAP> {
    let p: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let f: IShellItemImageFactory = SHCreateItemFromParsingName(PCWSTR(p.as_ptr()), None).ok()?;
        f.GetImage(SIZE { cx: 512, cy: 512 }, SIIGBF_BIGGERSIZEOK).ok()
    }
}

fn client_rect(h: HWND) -> RECT {
    let mut r = RECT::default();
    unsafe {
        let _ = GetClientRect(h, &mut r);
    }
    r
}

unsafe extern "system" fn preview_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_SIZE => {
            PREVIEW.with(|p| {
                if let Some(h) = p.borrow().as_ref().and_then(|p| p.handler.as_ref()) {
                    let _ = h.SetRect(&client_rect(hwnd));
                }
            });
            let _ = InvalidateRect(Some(hwnd), None, true);
            LRESULT(0)
        }
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let dc = BeginPaint(hwnd, &mut ps);
            let r = client_rect(hwnd);
            FillRect(dc, &r, HBRUSH(GetStockObject(DKGRAY_BRUSH).0));
            PREVIEW.with(|p| {
                if let Some(bmp) = p.borrow().as_ref().and_then(|p| p.thumbnail) {
                    let mem = CreateCompatibleDC(Some(dc));
                    let old = SelectObject(mem, bmp.into());
                    let mut info = BITMAP::default();
                    GetObjectW(bmp.into(), std::mem::size_of::<BITMAP>() as i32, Some(&mut info as *mut _ as *mut _));
                    // Fit the thumbnail in the window, centred, keeping its aspect.
                    let (bw, bh) = (info.bmWidth.max(1) as f32, info.bmHeight.max(1) as f32);
                    let (cw, ch) = ((r.right - r.left) as f32, (r.bottom - r.top) as f32);
                    let k = (cw / bw).min(ch / bh).min(1.0);
                    let (w, h) = ((bw * k) as i32, (bh * k) as i32);
                    let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8, ..Default::default() };
                    let _ = AlphaBlend(dc, (cw as i32 - w) / 2, (ch as i32 - h) / 2, w, h, mem, 0, 0, info.bmWidth, info.bmHeight, blend);
                    SelectObject(mem, old);
                    let _ = DeleteDC(mem);
                }
            });
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_CLOSE => {
            close_preview();
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

fn close_preview() {
    if let Some(p) = PREVIEW.with(|p| p.borrow_mut().take()) {
        unsafe {
            if let Some(h) = &p.handler {
                let _ = h.Unload();
            }
            if let Some(b) = p.thumbnail {
                let _ = DeleteObject(b.into());
            }
            let _ = DestroyWindow(p.hwnd);
        }
    }
}

/// Beside the launcher panel (right of it, or left when there's no room), as tall as 70% of the
/// screen's work area. Never activated, so the panel keeps focus and the keys.
fn preview_window() -> Option<HWND> {
    if let Some(h) = PREVIEW.with(|p| p.borrow().as_ref().map(|p| p.hwnd)) {
        return Some(h);
    }
    unsafe {
        let instance = GetModuleHandleW(None).ok()?;
        let class = w!("MooQuickLook");
        let wc = WNDCLASSW { lpfnWndProc: Some(preview_proc), hInstance: instance.into(), lpszClassName: class, hCursor: LoadCursorW(None, IDC_ARROW).ok()?, ..Default::default() };
        RegisterClassW(&wc);
        let panel = FindWindowW(w!("TishWindowsHost"), None).ok();
        let mut pr = RECT::default();
        if let Some(p) = panel {
            let _ = GetWindowRect(p, &mut pr);
        }
        let monitor = MonitorFromWindow(panel.unwrap_or_default(), MONITOR_DEFAULTTOPRIMARY);
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(monitor, &mut mi);
        let work = mi.rcWork;
        let h = ((work.bottom - work.top) as f32 * 0.7) as i32;
        let gap = 12;
        // Beside the panel, on the side with more room, narrowed to fit (on a small screen, over it).
        let (right, left) = (work.right - pr.right - gap, pr.left - gap - work.left);
        let w = ((h as f32 * 0.8) as i32).min(right.max(left)).max(280);
        let x = if right >= left { (pr.right + gap).min(work.right - w) } else { (pr.left - gap - w).max(work.left) };
        let y = pr.top.max(work.top).min(work.bottom - h);
        CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
            class,
            w!(""),
            WS_POPUP | WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | WS_CLIPCHILDREN,
            x,
            y,
            w,
            h,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .ok()
    }
}

/// Preview `path` (or switch the open preview to it); an empty path closes it. Whether a preview
/// is now open.
pub fn quick_look(path: &str) -> bool {
    if path.is_empty() {
        close_preview();
        return false;
    }
    let path = path.replace('/', "\\");
    if PREVIEW.with(|p| p.borrow().as_ref().is_some_and(|p| p.path == path)) {
        return true;
    }
    let Some(hwnd) = preview_window() else { return false };
    // Drop the previous file's handler or thumbnail, keep the window.
    if let Some(old) = PREVIEW.with(|p| p.borrow_mut().take()) {
        unsafe {
            if let Some(h) = &old.handler {
                let _ = h.Unload();
            }
            if let Some(b) = old.thumbnail {
                let _ = DeleteObject(b.into());
            }
        }
    }
    let handler = open_handler(&path);
    let thumbnail = if handler.is_none() { thumbnail(&path) } else { None };
    let title: Vec<u16> = std::path::Path::new(&path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default().encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let _ = SetWindowTextW(hwnd, PCWSTR(title.as_ptr()));
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        if let Some(h) = &handler {
            let _ = h.SetWindow(hwnd, &client_rect(hwnd));
            let _ = h.DoPreview();
        }
        let _ = InvalidateRect(Some(hwnd), None, true);
    }
    let shown = handler.is_some() || thumbnail.is_some();
    PREVIEW.with(|p| *p.borrow_mut() = Some(Preview { hwnd, path, handler, thumbnail }));
    if !shown {
        close_preview();
    }
    shown
}

pub fn quick_look_visible() -> bool {
    PREVIEW.with(|p| p.borrow().as_ref().is_some_and(|p| unsafe { IsWindowVisible(p.hwnd) }.as_bool()))
}
