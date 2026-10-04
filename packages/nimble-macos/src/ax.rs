//! Accessibility: move and resize the frontmost app's focused window (geometry in layout.rs), and
//! read the selected text in the focused field. Both need Nimble in System Settings › Privacy &
//! Security › Accessibility.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};
use objc2::{msg_send, MainThreadMarker};
use objc2_app_kit::{NSScreen, NSWorkspace};
use objc2_foundation::NSString;

use crate::layout::{self, Rect};

type CFTypeRef = *const c_void;

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrustedWithOptions(options: CFTypeRef) -> bool;
    fn AXUIElementCreateApplication(pid: i32) -> CFTypeRef;
    fn AXUIElementCreateSystemWide() -> CFTypeRef;
    fn AXUIElementCopyAttributeValue(element: CFTypeRef, attribute: CFTypeRef, value: *mut CFTypeRef) -> i32;
    fn AXUIElementSetAttributeValue(element: CFTypeRef, attribute: CFTypeRef, value: CFTypeRef) -> i32;
    fn AXValueCreate(kind: u32, value: *const c_void) -> CFTypeRef;
    fn AXValueGetValue(value: CFTypeRef, kind: u32, out: *mut c_void) -> bool;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(cf: CFTypeRef);
    fn CFHash(cf: CFTypeRef) -> usize;
    fn CFGetTypeID(cf: CFTypeRef) -> usize;
    fn CFStringGetTypeID() -> usize;
}

const AX_POINT: u32 = 1;
const AX_SIZE: u32 = 2;

#[repr(C)]
#[derive(Default)]
struct Pair {
    a: f64,
    b: f64,
}

/// A CF object we own; released on drop.
struct Owned(CFTypeRef);

impl Drop for Owned {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CFRelease(self.0) }
        }
    }
}

fn copy_attr(element: CFTypeRef, name: &str) -> Option<Owned> {
    let attr = NSString::from_str(name);
    let mut out: CFTypeRef = std::ptr::null();
    let err = unsafe { AXUIElementCopyAttributeValue(element, Retained::as_ptr(&attr) as CFTypeRef, &mut out) };
    (err == 0 && !out.is_null()).then(|| Owned(out))
}

fn set_pair(element: CFTypeRef, name: &str, kind: u32, a: f64, b: f64) -> i32 {
    let attr = NSString::from_str(name);
    let p = Pair { a, b };
    let v = Owned(unsafe { AXValueCreate(kind, &p as *const Pair as *const c_void) });
    unsafe { AXUIElementSetAttributeValue(element, Retained::as_ptr(&attr) as CFTypeRef, v.0) }
}

fn get_pair(element: CFTypeRef, name: &str, kind: u32) -> Option<(f64, f64)> {
    let v = copy_attr(element, name)?;
    let mut p = Pair::default();
    unsafe { AXValueGetValue(v.0, kind, &mut p as *mut Pair as *mut c_void) }.then_some((p.a, p.b))
}

/// Whether Nimble may use Accessibility. `prompt` asks macOS to show its permission dialog (once;
/// later calls only add Nimble to the list in System Settings).
pub fn trusted(prompt: bool) -> bool {
    unsafe {
        let Some(dict_cls) = AnyClass::get(c"NSDictionary") else { return false };
        let Some(num_cls) = AnyClass::get(c"NSNumber") else { return false };
        let yes: Retained<AnyObject> = msg_send![num_cls, numberWithBool: prompt];
        let key = NSString::from_str("AXTrustedCheckOptionPrompt");
        let opts: Retained<AnyObject> = msg_send![dict_cls, dictionaryWithObject: &*yes, forKey: &*key];
        AXIsProcessTrustedWithOptions(Retained::as_ptr(&opts) as CFTypeRef)
    }
}

const NEEDS_PERMISSION: &str = "Nimble needs Accessibility: System Settings › Privacy & Security › Accessibility";

/// Usable areas (Dock and menu bar left out) and full frames of every display, in Accessibility's
/// top-left coordinates.
fn screens() -> Vec<(Rect, Rect)> {
    let Some(mtm) = MainThreadMarker::new() else { return Vec::new() };
    let all = NSScreen::screens(mtm);
    let Some(primary) = all.iter().next() else { return Vec::new() };
    let top = primary.frame().size.height;
    let flip = |r: objc2_foundation::NSRect| Rect::new(r.origin.x, top - (r.origin.y + r.size.height), r.size.width, r.size.height);
    all.iter().map(|s| (flip(s.visibleFrame()), flip(s.frame()))).collect()
}

thread_local! {
    /// (pid, window hash) → the frame before Nimble last moved it, for Restore.
    static BEFORE: RefCell<HashMap<(i32, usize), Rect>> = RefCell::new(HashMap::new());
}

/// Apply `layout` (an id from layout::LAYOUTS) to the frontmost app's focused window. Returns a
/// short message; with `NIMBLE_SYSTEM_DRY_RUN` set it reports the frame without moving anything.
pub fn arrange(layout_id: &str) -> Result<String, String> {
    let title = layout::title(layout_id).ok_or_else(|| format!("unknown layout `{layout_id}`"))?;
    if !trusted(false) {
        trusted(true);
        return Err(NEEDS_PERMISSION.into());
    }
    let app = NSWorkspace::sharedWorkspace().frontmostApplication().ok_or("no frontmost app")?;
    let pid = app.processIdentifier();
    let name = app.localizedName().map(|s| s.to_string()).unwrap_or_default();
    if pid == std::process::id() as i32 {
        return Err("no window to arrange".into());
    }
    let ax_app = Owned(unsafe { AXUIElementCreateApplication(pid) });
    let win = copy_attr(ax_app.0, "AXFocusedWindow").or_else(|| copy_attr(ax_app.0, "AXMainWindow")).ok_or_else(|| format!("{name} has no window to arrange"))?;
    let (x, y) = get_pair(win.0, "AXPosition", AX_POINT).ok_or("cannot read the window position")?;
    let (w, h) = get_pair(win.0, "AXSize", AX_SIZE).ok_or("cannot read the window size")?;
    let cur = Rect::new(x, y, w, h);
    let all = screens();
    if all.is_empty() {
        return Err("no displays".into());
    }
    let frames: Vec<Rect> = all.iter().map(|s| s.1).collect();
    let i = layout::screen_for(cur, &frames);
    let key = (pid, unsafe { CFHash(win.0) });
    let target = match layout_id {
        "next-display" | "previous-display" => {
            if all.len() < 2 {
                return Err("only one display".into());
            }
            let n = all.len();
            let j = if layout_id == "next-display" { (i + 1) % n } else { (i + n - 1) % n };
            layout::move_to_screen(cur, all[i].0, all[j].0)
        }
        "restore" => BEFORE.with(|b| b.borrow().get(&key).copied()).ok_or_else(|| format!("nothing to restore for {name}"))?,
        id => layout::frame(id, all[i].0, cur).ok_or_else(|| format!("unknown layout `{id}`"))?,
    };
    if std::env::var_os("NIMBLE_SYSTEM_DRY_RUN").is_some() {
        return Ok(format!("dry run: {title} {name} {x},{y} {w}×{h} → {},{} {}×{}", target.x, target.y, target.w, target.h));
    }
    if layout_id != "restore" {
        BEFORE.with(|b| b.borrow_mut().insert(key, cur));
    }
    // Size first so the position fits on the new display, then size again: some apps clamp the
    // size to the display the window was on.
    set_pair(win.0, "AXSize", AX_SIZE, target.w, target.h);
    let err = set_pair(win.0, "AXPosition", AX_POINT, target.x, target.y);
    set_pair(win.0, "AXSize", AX_SIZE, target.w, target.h);
    if err != 0 {
        return Err(format!("{name} did not let its window move"));
    }
    Ok(String::new())
}

/// The selected text in the focused field of any app, or None (nothing selected, the app does
/// not expose it, or no permission).
pub fn selected_text() -> Option<String> {
    if !trusted(false) {
        return None;
    }
    let system = Owned(unsafe { AXUIElementCreateSystemWide() });
    let focused = copy_attr(system.0, "AXFocusedUIElement")?;
    let text = copy_attr(focused.0, "AXSelectedText")?;
    if unsafe { CFGetTypeID(text.0) != CFStringGetTypeID() } {
        return None;
    }
    let s: &NSString = unsafe { &*(text.0 as *const NSString) };
    let s = s.to_string();
    (!s.is_empty()).then_some(s)
}
