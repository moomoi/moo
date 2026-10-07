//! Window actions on another app's focused window through Accessibility: minimize, unminimize,
//! full screen, close and raise. Window frames (halves, maximize, …) are tish-macos's
//! `macos.accessibility.setFocusedWindowFrame`; these are the window states it does not reach.

use objc2::rc::Retained;
use objc2_foundation::NSString;
use std::ffi::c_void;

type CFTypeRef = *const c_void;

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXUIElementCreateApplication(pid: i32) -> CFTypeRef;
    fn AXUIElementCopyAttributeValue(element: CFTypeRef, attribute: CFTypeRef, value: *mut CFTypeRef) -> i32;
    fn AXUIElementSetAttributeValue(element: CFTypeRef, attribute: CFTypeRef, value: CFTypeRef) -> i32;
    fn AXUIElementPerformAction(element: CFTypeRef, action: CFTypeRef) -> i32;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(cf: CFTypeRef);
    fn CFArrayGetCount(array: CFTypeRef) -> isize;
    fn CFArrayGetValueAtIndex(array: CFTypeRef, index: isize) -> CFTypeRef;
    fn CFBooleanGetValue(boolean: CFTypeRef) -> bool;
    static kCFBooleanTrue: CFTypeRef;
    static kCFBooleanFalse: CFTypeRef;
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

fn ns(s: &str) -> Retained<NSString> {
    NSString::from_str(s)
}

fn copy_attr(element: CFTypeRef, name: &str) -> Option<Owned> {
    let attr = ns(name);
    let mut out: CFTypeRef = std::ptr::null();
    let err = unsafe { AXUIElementCopyAttributeValue(element, Retained::as_ptr(&attr) as CFTypeRef, &mut out) };
    (err == 0 && !out.is_null()).then(|| Owned(out))
}

fn set_bool(element: CFTypeRef, name: &str, on: bool) -> bool {
    let attr = ns(name);
    let v = unsafe { if on { kCFBooleanTrue } else { kCFBooleanFalse } };
    unsafe { AXUIElementSetAttributeValue(element, Retained::as_ptr(&attr) as CFTypeRef, v) == 0 }
}

fn get_bool(element: CFTypeRef, name: &str) -> bool {
    copy_attr(element, name).is_some_and(|v| unsafe { CFBooleanGetValue(v.0) })
}

fn press(element: CFTypeRef, action: &str) -> bool {
    let a = ns(action);
    unsafe { AXUIElementPerformAction(element, Retained::as_ptr(&a) as CFTypeRef) == 0 }
}

/// Run `action` on the app with `pid`: "minimize", "unminimize" (every minimized window),
/// "fullscreen" (toggles), "close" or "raise". Ok carries what happened.
pub fn window_action(pid: i32, name: &str, action: &str) -> Result<String, String> {
    if !unsafe { AXIsProcessTrusted() } {
        return Err("Moo needs Accessibility: System Settings › Privacy & Security › Accessibility".into());
    }
    let app = Owned(unsafe { AXUIElementCreateApplication(pid) });
    if action == "unminimize" {
        let Some(windows) = copy_attr(app.0, "AXWindows") else { return Err(format!("{name} has no windows")) };
        let mut n = 0;
        for i in 0..unsafe { CFArrayGetCount(windows.0) } {
            let w = unsafe { CFArrayGetValueAtIndex(windows.0, i) };
            if get_bool(w, "AXMinimized") && set_bool(w, "AXMinimized", false) {
                n += 1;
            }
        }
        return if n == 0 { Err(format!("{name} has no minimized windows")) } else { Ok(format!("Restored {n} {name} window{}", if n == 1 { "" } else { "s" })) };
    }
    let window = copy_attr(app.0, "AXFocusedWindow").or_else(|| copy_attr(app.0, "AXMainWindow"));
    let Some(w) = window else { return Err(format!("{name} has no open window")) };
    match action {
        "minimize" => set_bool(w.0, "AXMinimized", true).then(|| format!("Minimized {name}")).ok_or_else(|| format!("{name} would not minimize")),
        "fullscreen" => {
            let on = !get_bool(w.0, "AXFullScreen");
            set_bool(w.0, "AXFullScreen", on)
                .then(|| format!("{name} {}", if on { "is full screen" } else { "left full screen" }))
                .ok_or_else(|| format!("{name} cannot go full screen"))
        }
        "close" => {
            let Some(button) = copy_attr(w.0, "AXCloseButton") else { return Err(format!("{name}'s window cannot be closed")) };
            press(button.0, "AXPress").then(|| format!("Closed {name}'s window")).ok_or_else(|| format!("{name}'s window did not close"))
        }
        "raise" => press(w.0, "AXRaise").then(|| format!("Raised {name}")).ok_or_else(|| format!("{name} would not raise")),
        _ => Err(format!("unknown window action `{action}`")),
    }
}
