//! Hotkey specs name the modifiers macOS receives, as in every other Mac app. Keys remapped in
//! System Settings › Keyboard › Modifier Keys (Command ↔ Control, say) are already what macOS
//! reports, so a spec is registered exactly as written; the recorder records what macOS reports too.
//!
//! This module reads the enabled system shortcuts (Spotlight, input sources, ...): those win over
//! application hotkeys, so registering the same combination would succeed and never fire.

use std::ffi::{c_char, c_void, CStr, CString};

pub const CMD: u32 = 0x0100;
pub const SHIFT: u32 = 0x0200;
pub const OPT: u32 = 0x0800;
pub const CTRL: u32 = 0x1000;

pub fn spec_name(mods: u32, key: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for (bit, name) in [(CTRL, "ctrl"), (OPT, "alt"), (SHIFT, "shift"), (CMD, "cmd")] {
        if mods & bit != 0 {
            parts.push(name);
        }
    }
    parts.push(key);
    parts.join("+")
}

// ── CoreFoundation / IOKit ──────────────────────────────────────────────────

type CFTypeRef = *const c_void;
const UTF8: u32 = 0x0800_0100;
const SINT64: i64 = 4;

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFStringCreateWithCString(alloc: CFTypeRef, s: *const c_char, enc: u32) -> CFTypeRef;
    fn CFStringGetCString(s: CFTypeRef, buf: *mut c_char, len: isize, enc: u32) -> bool;
    fn CFRelease(v: CFTypeRef);
    fn CFGetTypeID(v: CFTypeRef) -> usize;
    fn CFStringGetTypeID() -> usize;
    fn CFNumberGetTypeID() -> usize;
    fn CFBooleanGetTypeID() -> usize;
    fn CFArrayGetTypeID() -> usize;
    fn CFDictionaryGetTypeID() -> usize;
    fn CFBooleanGetValue(b: CFTypeRef) -> bool;
    fn CFNumberGetValue(n: CFTypeRef, kind: i64, out: *mut c_void) -> bool;
    fn CFArrayGetCount(a: CFTypeRef) -> isize;
    fn CFArrayGetValueAtIndex(a: CFTypeRef, i: isize) -> CFTypeRef;
    fn CFDictionaryGetValue(d: CFTypeRef, k: CFTypeRef) -> CFTypeRef;
    fn CFDictionaryGetCount(d: CFTypeRef) -> isize;
    fn CFDictionaryGetKeysAndValues(d: CFTypeRef, keys: *mut CFTypeRef, values: *mut CFTypeRef);
    fn CFPreferencesCopyAppValue(key: CFTypeRef, app: CFTypeRef) -> CFTypeRef;
}

/// An owned CF object (from a Create/Copy call), released on drop.
struct Owned(CFTypeRef);

impl Drop for Owned {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CFRelease(self.0) }
        }
    }
}

fn cfstr(s: &str) -> Owned {
    let c = CString::new(s).unwrap();
    Owned(unsafe { CFStringCreateWithCString(std::ptr::null(), c.as_ptr(), UTF8) })
}

unsafe fn is(v: CFTypeRef, type_id: usize) -> bool {
    !v.is_null() && CFGetTypeID(v) == type_id
}

unsafe fn string(v: CFTypeRef) -> Option<String> {
    if !is(v, CFStringGetTypeID()) {
        return None;
    }
    let mut buf = [0 as c_char; 256];
    CFStringGetCString(v, buf.as_mut_ptr(), buf.len() as isize, UTF8)
        .then(|| CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned())
}

unsafe fn number(v: CFTypeRef) -> Option<i64> {
    if is(v, CFBooleanGetTypeID()) {
        return Some(CFBooleanGetValue(v) as i64);
    }
    if !is(v, CFNumberGetTypeID()) {
        return None;
    }
    let mut n = 0i64;
    CFNumberGetValue(v, SINT64, &mut n as *mut i64 as *mut c_void).then_some(n)
}

unsafe fn get(d: CFTypeRef, key: &str) -> CFTypeRef {
    if !is(d, CFDictionaryGetTypeID()) {
        return std::ptr::null();
    }
    CFDictionaryGetValue(d, cfstr(key).0)
}

unsafe fn array(a: CFTypeRef) -> Vec<CFTypeRef> {
    if !is(a, CFArrayGetTypeID()) {
        return Vec::new();
    }
    (0..CFArrayGetCount(a)).map(|i| CFArrayGetValueAtIndex(a, i)).collect()
}

/// A system shortcut as stored in `com.apple.symbolichotkeys`: id, enabled, `[char, key code, flags]`.
pub type Shortcut = (i64, bool, Vec<i64>);

/// Space-bar shortcuts as macOS ships them. The preferences only hold shortcuts the user changed,
/// so an id missing there still has its default binding.
const DEFAULT_SHORTCUTS: [(i64, &str, i64); 4] = [
    (60, "Select the previous input source", 1 << 18),
    (61, "Select next source in Input menu", (1 << 18) | (1 << 19)),
    (64, "Show Spotlight search", 1 << 20),
    (65, "Show Finder search window", (1 << 20) | (1 << 19)),
];

/// The stored system shortcuts.
pub fn stored_shortcuts() -> Vec<Shortcut> {
    let mut out = Vec::new();
    unsafe {
        let all = Owned(CFPreferencesCopyAppValue(
            cfstr("AppleSymbolicHotKeys").0,
            cfstr("com.apple.symbolichotkeys").0,
        ));
        if !is(all.0, CFDictionaryGetTypeID()) {
            return out;
        }
        let n = CFDictionaryGetCount(all.0).max(0) as usize;
        let mut keys = vec![std::ptr::null(); n];
        let mut values = vec![std::ptr::null(); n];
        CFDictionaryGetKeysAndValues(all.0, keys.as_mut_ptr(), values.as_mut_ptr());
        for (k, v) in keys.into_iter().zip(values) {
            let Some(id) = string(k).and_then(|s| s.parse().ok()) else { continue };
            let enabled = number(get(v, "enabled")) == Some(1);
            let params = array(get(get(v, "value"), "parameters")).into_iter().filter_map(|p| number(p)).collect();
            out.push((id, enabled, params));
        }
    }
    out
}

/// The system shortcut that owns `(key_code, carbon mods)`, given the stored shortcuts.
pub fn conflict(stored: &[Shortcut], key_code: u32, mods: u32) -> Option<String> {
    let flags = [(SHIFT, 1i64 << 17), (CTRL, 1 << 18), (OPT, 1 << 19), (CMD, 1 << 20)]
        .iter()
        .filter(|(m, _)| mods & m != 0)
        .map(|(_, f)| f)
        .sum::<i64>();
    let name = |id: i64| {
        DEFAULT_SHORTCUTS.iter().find(|d| d.0 == id).map_or(format!("system shortcut {id}"), |d| d.1.to_string())
    };
    let hit = |p: &[i64]| p.len() >= 3 && p[1] == key_code as i64 && p[2] & 0x1E_0000 == flags;
    for (id, enabled, params) in stored {
        if *enabled && hit(params) {
            return Some(name(*id));
        }
    }
    for (id, _, f) in DEFAULT_SHORTCUTS {
        if key_code == 49 && f == flags && !stored.iter().any(|s| s.0 == id) {
            return Some(name(id));
        }
    }
    None
}

pub fn system_shortcut(key_code: u32, mods: u32) -> Option<String> {
    conflict(&stored_shortcuts(), key_code, mods)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(spec_name(CTRL | CMD, "space"), "ctrl+cmd+space");
    }

    #[test]
    fn system_shortcuts_block_their_combination() {
        let spotlight_off = vec![(64, false, vec![32, 49, 1 << 20])];
        assert_eq!(conflict(&spotlight_off, 49, CMD), None);
        // Missing from the preferences means the default binding is live.
        assert_eq!(conflict(&[], 49, CMD).as_deref(), Some("Show Spotlight search"));
        assert_eq!(conflict(&[], 49, CTRL).as_deref(), Some("Select the previous input source"));
        assert_eq!(conflict(&[], 49, OPT), None);
        // A user-rebound shortcut is matched on its stored binding; caps-lock/fn flag bits are ignored.
        let rebound = vec![(64, true, vec![32, 49, (1 << 19) | (1 << 16)])];
        assert_eq!(conflict(&rebound, 49, OPT).as_deref(), Some("Show Spotlight search"));
        assert_eq!(conflict(&rebound, 49, CMD), None);
    }

    /// Reads this Mac's real settings; only checks that the call works.
    #[test]
    fn reads_live_system_shortcuts() {
        eprintln!("stored system shortcuts {:?}; ctrl+space owner {:?}", stored_shortcuts().len(), system_shortcut(49, CTRL));
    }
}
