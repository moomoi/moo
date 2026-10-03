//! Hotkey specs name keys as printed on the keyboard. System Settings › Keyboard › Modifier Keys
//! can remap them per keyboard (e.g. Command ↔ Control), and the remap happens below the event
//! system: a physical Command key then *produces* Control, and a Carbon hotkey must be registered
//! for Control. This module reads the mapping of each connected keyboard and translates.
//!
//! It also reads the enabled system shortcuts (Spotlight, input sources, ...): those win over
//! application hotkeys, so registering the same combination would succeed and never fire.

use std::ffi::{c_char, c_void, CStr, CString};

pub const CMD: u32 = 0x0100;
pub const SHIFT: u32 = 0x0200;
pub const OPT: u32 = 0x0800;
pub const CTRL: u32 = 0x1000;
const MODS: [u32; 4] = [CTRL, OPT, SHIFT, CMD];

/// HID usage (page 7) of the left and right key for a Carbon modifier bit.
fn usages(m: u32) -> [u64; 2] {
    let (l, r) = match m {
        CTRL => (0xE0, 0xE4),
        SHIFT => (0xE1, 0xE5),
        OPT => (0xE2, 0xE6),
        _ => (0xE3, 0xE7),
    };
    [0x7_0000_0000 | l, 0x7_0000_0000 | r]
}

/// The Carbon modifier bit a HID usage produces, if it is a modifier key.
fn class(usage: u64) -> Option<u32> {
    match usage {
        0x7_0000_00E0 | 0x7_0000_00E4 => Some(CTRL),
        0x7_0000_00E1 | 0x7_0000_00E5 => Some(SHIFT),
        0x7_0000_00E2 | 0x7_0000_00E6 => Some(OPT),
        0x7_0000_00E3 | 0x7_0000_00E7 => Some(CMD),
        _ => None,
    }
}

/// Logical modifier sets the physical modifiers `physical` can produce, given each keyboard's
/// `(src, dst)` usage pairs. Left and right keys may map differently, so one spec can need several
/// registrations. A keyboard on which some named key produces no modifier contributes nothing.
pub fn logical_combos(physical: u32, keyboards: &[Vec<(u64, u64)>]) -> Vec<u32> {
    let identity = [Vec::new()];
    let keyboards = if keyboards.is_empty() { &identity[..] } else { keyboards };
    let mut out = Vec::new();
    for pairs in keyboards {
        let map = |u: u64| pairs.iter().find(|(s, _)| *s == u).map_or(u, |(_, d)| *d);
        let mut combos = vec![0u32];
        for m in MODS.iter().copied().filter(|m| physical & m != 0) {
            let mut options: Vec<u32> = usages(m).iter().filter_map(|&u| class(map(u))).collect();
            options.dedup();
            combos = combos.iter().flat_map(|c| options.iter().map(move |o| c | o)).collect();
        }
        for c in combos {
            if !out.contains(&c) {
                out.push(c);
            }
        }
    }
    out
}

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

pub fn symbols(mods: u32, key: &str) -> String {
    let mut s = String::new();
    for (bit, sym) in [(CTRL, "⌃"), (OPT, "⌥"), (SHIFT, "⇧"), (CMD, "⌘")] {
        if mods & bit != 0 {
            s.push_str(sym);
        }
    }
    let mut k = key.chars();
    s.extend(k.next().map(|c| c.to_ascii_uppercase()));
    s.push_str(k.as_str());
    s
}

// ── CoreFoundation / IOKit ──────────────────────────────────────────────────

type CFTypeRef = *const c_void;
const UTF8: u32 = 0x0800_0100;
const SINT64: i64 = 4;

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    static kCFPreferencesAnyApplication: CFTypeRef;
    static kCFPreferencesCurrentUser: CFTypeRef;
    static kCFPreferencesCurrentHost: CFTypeRef;
    static kCFPreferencesAnyHost: CFTypeRef;
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
    fn CFSetGetCount(s: CFTypeRef) -> isize;
    fn CFSetGetValues(s: CFTypeRef, out: *mut CFTypeRef);
    fn CFPreferencesCopyKeyList(app: CFTypeRef, user: CFTypeRef, host: CFTypeRef) -> CFTypeRef;
    fn CFPreferencesCopyValue(key: CFTypeRef, app: CFTypeRef, user: CFTypeRef, host: CFTypeRef) -> CFTypeRef;
    fn CFPreferencesCopyAppValue(key: CFTypeRef, app: CFTypeRef) -> CFTypeRef;
}

#[link(name = "IOKit", kind = "framework")]
extern "C" {
    fn IOHIDManagerCreate(alloc: CFTypeRef, options: u32) -> CFTypeRef;
    fn IOHIDManagerSetDeviceMatching(m: CFTypeRef, matching: CFTypeRef);
    fn IOHIDManagerCopyDevices(m: CFTypeRef) -> CFTypeRef;
    fn IOHIDDeviceConformsTo(d: CFTypeRef, page: u32, usage: u32) -> bool;
    fn IOHIDDeviceGetProperty(d: CFTypeRef, key: CFTypeRef) -> CFTypeRef;
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

/// `(vendor, product)` of every connected keyboard (HID usage page 1, usage 6).
pub fn connected_keyboards() -> Vec<(i64, i64)> {
    unsafe {
        let mgr = Owned(IOHIDManagerCreate(std::ptr::null(), 0));
        if mgr.0.is_null() {
            return Vec::new();
        }
        IOHIDManagerSetDeviceMatching(mgr.0, std::ptr::null());
        let set = Owned(IOHIDManagerCopyDevices(mgr.0));
        if set.0.is_null() {
            return Vec::new();
        }
        let mut devices = vec![std::ptr::null(); CFSetGetCount(set.0).max(0) as usize];
        CFSetGetValues(set.0, devices.as_mut_ptr());
        let (vk, pk) = (cfstr("VendorID"), cfstr("ProductID"));
        let mut out = Vec::new();
        for d in devices {
            if !IOHIDDeviceConformsTo(d, 1, 6) {
                continue;
            }
            let id = (
                number(IOHIDDeviceGetProperty(d, vk.0)).unwrap_or(0),
                number(IOHIDDeviceGetProperty(d, pk.0)).unwrap_or(0),
            );
            if !out.contains(&id) {
                out.push(id);
            }
        }
        out
    }
}

/// Saved modifier mappings by keyboard: `com.apple.keyboard.modifiermapping.<vendor>-<product>-0`
/// in the global domain, current host first.
pub fn saved_mappings() -> Vec<((i64, i64), Vec<(u64, u64)>)> {
    const PREFIX: &str = "com.apple.keyboard.modifiermapping.";
    let mut out: Vec<((i64, i64), Vec<(u64, u64)>)> = Vec::new();
    unsafe {
        for host in [kCFPreferencesCurrentHost, kCFPreferencesAnyHost] {
            let keys = Owned(CFPreferencesCopyKeyList(kCFPreferencesAnyApplication, kCFPreferencesCurrentUser, host));
            for k in array(keys.0) {
                let Some(name) = string(k) else { continue };
                let Some(rest) = name.strip_prefix(PREFIX) else { continue };
                let mut ids = rest.split('-').map(|p| p.parse::<i64>().ok());
                let (Some(Some(vendor)), Some(Some(product))) = (ids.next(), ids.next()) else { continue };
                if out.iter().any(|(id, _)| *id == (vendor, product)) {
                    continue;
                }
                let value =
                    Owned(CFPreferencesCopyValue(k, kCFPreferencesAnyApplication, kCFPreferencesCurrentUser, host));
                let pairs = array(value.0)
                    .into_iter()
                    .filter_map(|m| {
                        let src = number(get(m, "HIDKeyboardModifierMappingSrc"))?;
                        let dst = number(get(m, "HIDKeyboardModifierMappingDst"))?;
                        Some((src as u64, dst as u64))
                    })
                    .collect();
                out.push(((vendor, product), pairs));
            }
        }
    }
    out
}

/// The mapping of each connected keyboard (none saved means unmapped). If no keyboard can be
/// found, every saved mapping is used.
pub fn active_mappings() -> Vec<Vec<(u64, u64)>> {
    let saved = saved_mappings();
    let connected = connected_keyboards();
    if connected.is_empty() {
        return saved.into_iter().map(|(_, p)| p).collect();
    }
    connected
        .iter()
        .map(|id| saved.iter().find(|(s, _)| s == id).map(|(_, p)| p.clone()).unwrap_or_default())
        .collect()
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

    const L_CTRL: u64 = 0x7_0000_00E0;
    const L_CMD: u64 = 0x7_0000_00E3;
    const R_CTRL: u64 = 0x7_0000_00E4;
    const R_CMD: u64 = 0x7_0000_00E7;
    const CAPS: u64 = 0x7_0000_0039;

    fn swapped() -> Vec<(u64, u64)> {
        vec![(L_CTRL, L_CMD), (L_CMD, L_CTRL), (R_CMD, R_CTRL), (R_CTRL, R_CMD)]
    }

    #[test]
    fn unmapped_keyboards_keep_the_spec() {
        assert_eq!(logical_combos(CMD, &[]), vec![CMD]);
        assert_eq!(logical_combos(CMD | SHIFT, &[Vec::new()]), vec![CMD | SHIFT]);
    }

    #[test]
    fn command_control_swap_turns_cmd_into_ctrl_and_back() {
        assert_eq!(logical_combos(CMD, &[swapped()]), vec![CTRL]);
        assert_eq!(logical_combos(CTRL | OPT, &[swapped()]), vec![CMD | OPT]);
    }

    #[test]
    fn sides_and_keyboards_that_differ_need_several_registrations() {
        let left_only = vec![(L_CMD, L_CTRL)];
        let mut got = logical_combos(CMD, &[left_only]);
        got.sort();
        assert_eq!(got, vec![CMD, CTRL]);
        let mut got = logical_combos(CMD, &[swapped(), Vec::new()]);
        got.sort();
        assert_eq!(got, vec![CMD, CTRL]);
    }

    #[test]
    fn a_key_mapped_to_a_non_modifier_cannot_be_used() {
        let both_to_caps = vec![(L_CMD, CAPS), (R_CMD, CAPS)];
        assert!(logical_combos(CMD, &[both_to_caps]).is_empty());
    }

    #[test]
    fn names() {
        assert_eq!(spec_name(CTRL | CMD, "space"), "ctrl+cmd+space");
        assert_eq!(symbols(CMD | SHIFT, "space"), "⇧⌘Space");
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

    /// Reads this Mac's real settings; only checks that the calls work and agree with each other.
    #[test]
    fn reads_live_keyboard_settings() {
        let connected = connected_keyboards();
        let active = active_mappings();
        assert!(connected.is_empty() || active.len() == connected.len());
        eprintln!("connected keyboards {connected:?}, saved {:?}", saved_mappings().iter().map(|(id, p)| (id, p.len())).collect::<Vec<_>>());
        eprintln!("physical cmd+space registers {:?}", logical_combos(CMD, &active).iter().map(|m| spec_name(*m, "space")).collect::<Vec<_>>());
        eprintln!("stored system shortcuts {:?}; ctrl+space owner {:?}", stored_shortcuts().len(), system_shortcut(49, CTRL));
    }
}
