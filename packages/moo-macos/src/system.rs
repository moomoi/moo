//! System commands: lock, sleep, restart / shut down / log out (Apple Events to loginwindow),
//! empty Trash (Apple Event to Finder), dark mode, output volume and mute (CoreAudio), eject
//! disks, screen saver, and running apps (list with memory, switch, hide, quit, force quit).

use std::ffi::{c_char, c_void, CString};

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};
use objc2::msg_send;
use objc2_app_kit::{NSApplicationActivationOptions, NSApplicationActivationPolicy, NSRunningApplication, NSWorkspace};
use objc2_foundation::{NSArray, NSString, NSURL};

extern "C" {
    fn dlopen(path: *const c_char, mode: i32) -> *mut c_void;
    fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
    fn proc_pid_rusage(pid: i32, flavor: i32, buffer: *mut c_void) -> i32;
}

const RTLD_LAZY: i32 = 1;

fn symbol(lib: &str, name: &str) -> Option<*mut c_void> {
    let (l, n) = (CString::new(lib).ok()?, CString::new(name).ok()?);
    unsafe {
        let h = dlopen(l.as_ptr(), RTLD_LAZY);
        if h.is_null() {
            return None;
        }
        let s = dlsym(h, n.as_ptr());
        (!s.is_null()).then_some(s)
    }
}

pub fn lock_screen() -> Result<(), String> {
    let f = symbol("/System/Library/PrivateFrameworks/login.framework/Versions/Current/login", "SACLockScreenImmediate")
        .ok_or("cannot find SACLockScreenImmediate")?;
    let f: extern "C" fn() -> i32 = unsafe { std::mem::transmute(f) };
    f();
    Ok(())
}

fn pmset(arg: &str) -> Result<(), String> {
    let st = std::process::Command::new("/usr/bin/pmset").arg(arg).status().map_err(|e| e.to_string())?;
    if st.success() {
        Ok(())
    } else {
        Err(format!("pmset {arg} failed"))
    }
}

pub fn sleep() -> Result<(), String> {
    pmset("sleepnow")
}

pub fn sleep_displays() -> Result<(), String> {
    pmset("displaysleepnow")
}

const fn fourcc(s: &[u8; 4]) -> u32 {
    ((s[0] as u32) << 24) | ((s[1] as u32) << 16) | ((s[2] as u32) << 8) | (s[3] as u32)
}

/// Send Apple Event `class`/`id` to the app `bundle`. `wait` waits up to that many seconds for the
/// reply; 0 sends without waiting.
fn apple_event(bundle: &str, class: &[u8; 4], id: &[u8; 4], wait: f64) -> Result<(), String> {
    let cls = AnyClass::get(c"NSAppleEventDescriptor").ok_or("no NSAppleEventDescriptor")?;
    let bundle = NSString::from_str(bundle);
    unsafe {
        let target: Option<Retained<AnyObject>> = msg_send![cls, descriptorWithBundleIdentifier: &*bundle];
        let target = target.ok_or("bad target")?;
        let ev: Option<Retained<AnyObject>> = msg_send![
            cls,
            appleEventWithEventClass: fourcc(class),
            eventID: fourcc(id),
            targetDescriptor: &*target,
            returnID: -1i16,
            transactionID: 0i32
        ];
        let ev = ev.ok_or("cannot create the Apple Event")?;
        // kAENoReply = 1, kAEWaitReply = 3
        let options: usize = if wait > 0.0 { 3 } else { 1 };
        let mut err: *mut AnyObject = std::ptr::null_mut();
        let reply: Option<Retained<AnyObject>> = msg_send![&*ev, sendEventWithOptions: options, timeout: wait.max(1.0), error: &mut err];
        if reply.is_none() && !err.is_null() {
            let desc: Retained<NSString> = msg_send![err, localizedDescription];
            return Err(desc.to_string());
        }
    }
    Ok(())
}

pub fn restart() -> Result<(), String> {
    apple_event("com.apple.loginwindow", b"aevt", b"rest", 0.0)
}

pub fn shut_down() -> Result<(), String> {
    apple_event("com.apple.loginwindow", b"aevt", b"shut", 0.0)
}

pub fn log_out() -> Result<(), String> {
    apple_event("com.apple.loginwindow", b"aevt", b"rlgo", 0.0)
}

/// Asks Finder (macOS asks the user once to allow Moo to control Finder). Blocks: worker thread.
pub fn empty_trash() -> Result<(), String> {
    apple_event("com.apple.finder", b"fndr", b"empt", 60.0)
}

pub fn screen_saver() -> bool {
    crate::mac::launch("/System/Library/CoreServices/ScreenSaverEngine.app")
}

// ── Appearance ──────────────────────────────────────────────────────────────

const SKYLIGHT: &str = "/System/Library/PrivateFrameworks/SkyLight.framework/SkyLight";

pub fn dark_mode() -> Option<bool> {
    let f = symbol(SKYLIGHT, "SLSGetAppearanceThemeLegacy")?;
    let f: extern "C" fn() -> bool = unsafe { std::mem::transmute(f) };
    Some(f())
}

pub fn set_dark_mode(on: bool) -> Result<(), String> {
    let f = symbol(SKYLIGHT, "SLSSetAppearanceThemeLegacy").ok_or("cannot change the appearance on this macOS")?;
    let f: extern "C" fn(bool) = unsafe { std::mem::transmute(f) };
    f(on);
    Ok(())
}

// ── Sound ───────────────────────────────────────────────────────────────────

#[repr(C)]
struct PropertyAddress {
    selector: u32,
    scope: u32,
    element: u32,
}

#[link(name = "CoreAudio", kind = "framework")]
extern "C" {
    fn AudioObjectGetPropertyData(obj: u32, addr: *const PropertyAddress, qsize: u32, q: *const c_void, size: *mut u32, data: *mut c_void) -> i32;
    fn AudioObjectSetPropertyData(obj: u32, addr: *const PropertyAddress, qsize: u32, q: *const c_void, size: u32, data: *const c_void) -> i32;
}

const SYSTEM_OBJECT: u32 = 1;

fn output_device() -> Result<u32, String> {
    let addr = PropertyAddress { selector: fourcc(b"dOut"), scope: fourcc(b"glob"), element: 0 };
    let mut dev: u32 = 0;
    let mut size = 4u32;
    let st = unsafe { AudioObjectGetPropertyData(SYSTEM_OBJECT, &addr, 0, std::ptr::null(), &mut size, &mut dev as *mut u32 as *mut c_void) };
    if st != 0 || dev == 0 {
        return Err("no sound output device".into());
    }
    Ok(dev)
}

/// Output volume 0–100 and mute.
pub fn volume() -> Result<(f64, bool), String> {
    let dev = output_device()?;
    let vol_addr = PropertyAddress { selector: fourcc(b"vmvc"), scope: fourcc(b"outp"), element: 0 };
    let mut v: f32 = 0.0;
    let mut size = 4u32;
    let st = unsafe { AudioObjectGetPropertyData(dev, &vol_addr, 0, std::ptr::null(), &mut size, &mut v as *mut f32 as *mut c_void) };
    if st != 0 {
        return Err("the output device has no volume control".into());
    }
    let mute_addr = PropertyAddress { selector: fourcc(b"mute"), scope: fourcc(b"outp"), element: 0 };
    let mut m: u32 = 0;
    let mut size = 4u32;
    let st = unsafe { AudioObjectGetPropertyData(dev, &mute_addr, 0, std::ptr::null(), &mut size, &mut m as *mut u32 as *mut c_void) };
    Ok(((v as f64 * 100.0).round(), st == 0 && m != 0))
}

pub fn set_volume(percent: f64) -> Result<(), String> {
    let dev = output_device()?;
    let addr = PropertyAddress { selector: fourcc(b"vmvc"), scope: fourcc(b"outp"), element: 0 };
    let v = (percent.clamp(0.0, 100.0) / 100.0) as f32;
    let st = unsafe { AudioObjectSetPropertyData(dev, &addr, 0, std::ptr::null(), 4, &v as *const f32 as *const c_void) };
    if st != 0 {
        return Err("cannot set the volume of this output device".into());
    }
    if percent > 0.0 && volume().is_ok_and(|(_, muted)| muted) {
        set_mute(false)?;
    }
    Ok(())
}

pub fn set_mute(on: bool) -> Result<(), String> {
    let dev = output_device()?;
    let addr = PropertyAddress { selector: fourcc(b"mute"), scope: fourcc(b"outp"), element: 0 };
    let m: u32 = on as u32;
    let st = unsafe { AudioObjectSetPropertyData(dev, &addr, 0, std::ptr::null(), 4, &m as *const u32 as *const c_void) };
    if st != 0 {
        return Err("this output device cannot be muted".into());
    }
    Ok(())
}

// ── Disks ───────────────────────────────────────────────────────────────────

/// Unmount and eject every ejectable volume. Returns the names ejected and the failures. Blocks.
pub fn eject_all() -> (Vec<String>, Vec<String>) {
    let mut ejected = Vec::new();
    let mut failed = Vec::new();
    unsafe {
        let Some(fm_cls) = AnyClass::get(c"NSFileManager") else { return (ejected, failed) };
        let fm: Retained<AnyObject> = msg_send![fm_cls, defaultManager];
        let keys = NSArray::from_retained_slice(&[NSString::from_str("NSURLVolumeIsEjectableKey"), NSString::from_str("NSURLVolumeIsRemovableKey"), NSString::from_str("NSURLVolumeIsInternalKey"), NSString::from_str("NSURLVolumeLocalizedNameKey")]);
        // NSVolumeEnumerationSkipHiddenVolumes = 2
        let urls: Option<Retained<NSArray<NSURL>>> = msg_send![&*fm, mountedVolumeURLsIncludingResourceValuesForKeys: &*keys, options: 2usize];
        let Some(urls) = urls else { return (ejected, failed) };
        let ws = NSWorkspace::sharedWorkspace();
        for i in 0..urls.count() {
            let url = urls.objectAtIndex(i);
            let flag = |key: &str| -> bool {
                let mut v: *mut AnyObject = std::ptr::null_mut();
                let k = NSString::from_str(key);
                let ok: bool = msg_send![&*url, getResourceValue: &mut v, forKey: &*k, error: std::ptr::null_mut::<*mut AnyObject>()];
                ok && !v.is_null() && { let b: bool = msg_send![v, boolValue]; b }
            };
            let path = url.path().map(|p| p.to_string()).unwrap_or_default();
            if path == "/" || !(flag("NSURLVolumeIsEjectableKey") || flag("NSURLVolumeIsRemovableKey") || (!flag("NSURLVolumeIsInternalKey") && path.starts_with("/Volumes/"))) {
                continue;
            }
            let name = std::path::Path::new(&path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(path.clone());
            let mut err: *mut AnyObject = std::ptr::null_mut();
            let ok: bool = msg_send![&*ws, unmountAndEjectDeviceAtURL: &*url, error: &mut err];
            if ok {
                ejected.push(name);
            } else {
                let why = if err.is_null() { "busy".to_string() } else { let d: Retained<NSString> = msg_send![err, localizedDescription]; d.to_string() };
                failed.push(format!("{name}: {why}"));
            }
        }
    }
    (ejected, failed)
}

// ── Running apps ────────────────────────────────────────────────────────────

pub struct RunningApp {
    pub pid: i32,
    pub name: String,
    pub path: String,
    pub bundle_id: String,
    pub active: bool,
    pub hidden: bool,
    /// Physical memory footprint in bytes (what Activity Monitor calls Memory).
    pub memory: u64,
}

fn footprint(pid: i32) -> u64 {
    // rusage_info_v0: 16-byte uuid, then u64s; ri_phys_footprint is the 8th.
    let mut buf = [0u64; 16];
    let st = unsafe { proc_pid_rusage(pid, 0, buf.as_mut_ptr() as *mut c_void) };
    if st == 0 {
        buf[2 + 7]
    } else {
        0
    }
}

/// Apps in the Dock (regular activation policy), Moo excluded, by memory use.
pub fn running_apps() -> Vec<RunningApp> {
    let me = std::process::id() as i32;
    let apps = NSWorkspace::sharedWorkspace().runningApplications();
    let mut out: Vec<RunningApp> = (0..apps.count())
        .map(|i| apps.objectAtIndex(i))
        .filter(|a| a.activationPolicy() == NSApplicationActivationPolicy::Regular && a.processIdentifier() != me)
        .map(|a| {
            let pid = a.processIdentifier();
            RunningApp {
                pid,
                name: a.localizedName().map(|s| s.to_string()).unwrap_or_default(),
                path: a.bundleURL().and_then(|u| u.path()).map(|p| p.to_string()).unwrap_or_default(),
                bundle_id: a.bundleIdentifier().map(|s| s.to_string()).unwrap_or_default(),
                active: a.isActive(),
                hidden: a.isHidden(),
                memory: footprint(pid),
            }
        })
        .collect();
    out.sort_by(|a, b| b.memory.cmp(&a.memory));
    out
}

fn app_by_pid(pid: i32) -> Option<Retained<NSRunningApplication>> {
    NSRunningApplication::runningApplicationWithProcessIdentifier(pid)
}

/// `switch`, `hide`, `unhide`, `quit` or `force-quit` the app with `pid`. Returns a short message;
/// with `MOO_SYSTEM_DRY_RUN` set it only reports what it would do.
pub fn app_action(pid: i32, action: &str) -> Result<String, String> {
    let a = app_by_pid(pid).ok_or_else(|| format!("no app with pid {pid}"))?;
    let name = a.localizedName().map(|s| s.to_string()).unwrap_or_default();
    if !matches!(action, "switch" | "hide" | "unhide" | "quit" | "force-quit") {
        return Err(format!("unknown app action `{action}`"));
    }
    if std::env::var_os("MOO_SYSTEM_DRY_RUN").is_some() {
        return Ok(format!("dry run: {action} {name}"));
    }
    let ok = match action {
        "switch" => {
            a.unhide();
            #[allow(deprecated)]
            a.activateWithOptions(NSApplicationActivationOptions::ActivateAllWindows)
        }
        // On macOS 26 both return NO even when the app hides or shows, so the result is ignored.
        "hide" => {
            a.hide();
            true
        }
        "unhide" => {
            a.unhide();
            true
        }
        "quit" => a.terminate(),
        _ => a.forceTerminate(),
    };
    if !ok {
        return Err(format!("{name} did not {}", action.replace('-', " ")));
    }
    Ok(match action {
        "hide" => format!("Hid {name}"),
        "unhide" => format!("Showing {name}"),
        "quit" => format!("Asked {name} to quit"),
        "force-quit" => format!("Force quit {name}"),
        _ => String::new(),
    })
}

/// Quit every app in the Dock but Finder (and Moo). Returns how many were asked.
pub fn quit_all() -> usize {
    let mut n = 0;
    for a in running_apps().iter().filter(|a| a.bundle_id != "com.apple.finder") {
        if app_by_pid(a.pid).is_some_and(|x| x.terminate()) {
            n += 1;
        }
    }
    n
}

/// Hide every app in the Dock (shows the desktop).
pub fn hide_all() -> usize {
    let mut n = 0;
    for a in running_apps() {
        if let Some(x) = app_by_pid(a.pid) {
            x.hide();
            n += 1;
        }
    }
    n
}

/// Commands that wait on another app or the disks; run them off the main thread.
pub fn blocks(id: &str) -> bool {
    matches!(id, "empty-trash" | "eject")
}

fn toggle_arg(arg: &str, current: bool) -> bool {
    match arg {
        "on" => true,
        "off" => false,
        _ => !current,
    }
}

/// Run system command `id`; `arg` is `on`/`off`/`toggle` for `dark-mode` and `mute`, and a
/// percentage, `up` or `down` for `volume` (empty reports the level). Returns a short message.
/// With `MOO_SYSTEM_DRY_RUN` set, known commands only report what they would do.
pub fn run(id: &str, arg: &str) -> Result<String, String> {
    let arg = arg.trim().to_lowercase();
    if std::env::var_os("MOO_SYSTEM_DRY_RUN").is_some() {
        return match id {
            "lock" | "sleep" | "sleep-displays" | "restart" | "shut-down" | "log-out" | "empty-trash" | "screen-saver" | "dark-mode"
            | "mute" | "volume" | "eject" | "quit-all" | "hide-all" => Ok(format!("dry run: {id} {arg}").trim_end().to_string()),
            _ => Err(format!("unknown system command `{id}`")),
        };
    }
    match id {
        "lock" => lock_screen().map(|_| String::new()),
        "sleep" => sleep().map(|_| String::new()),
        "sleep-displays" => sleep_displays().map(|_| String::new()),
        "restart" => restart().map(|_| "Restarting".into()),
        "shut-down" => shut_down().map(|_| "Shutting down".into()),
        "log-out" => log_out().map(|_| "Logging out".into()),
        "empty-trash" => empty_trash().map(|_| "Trash emptied".into()),
        "screen-saver" => screen_saver().then(String::new).ok_or_else(|| "Could not start the screen saver".into()),
        "dark-mode" => {
            let on = toggle_arg(&arg, dark_mode().ok_or("cannot read the appearance")?);
            set_dark_mode(on)?;
            Ok(if on { "Dark mode on" } else { "Dark mode off" }.into())
        }
        "mute" => {
            let on = toggle_arg(&arg, volume()?.1);
            set_mute(on)?;
            Ok(if on { "Sound muted" } else { "Sound on" }.into())
        }
        "volume" => {
            let (v, muted) = volume()?;
            let target = match arg.trim_end_matches('%') {
                "" => return Ok(if muted { format!("Volume {v}% (muted)") } else { format!("Volume {v}%") }),
                "up" => v + 10.0,
                "down" => v - 10.0,
                n => n.parse::<f64>().map_err(|_| format!("not a volume: {arg}"))?,
            }
            .clamp(0.0, 100.0)
            .round();
            set_volume(target)?;
            Ok(format!("Volume {target}%"))
        }
        "eject" => {
            let (ejected, failed) = eject_all();
            match (ejected.is_empty(), failed.is_empty()) {
                (true, true) => Ok("No disks to eject".into()),
                (_, true) => Ok(format!("Ejected {}", ejected.join(", "))),
                _ => Err(format!("Could not eject {}", failed.join("; "))),
            }
        }
        "quit-all" => Ok(format!("Asked {} apps to quit", quit_all())),
        "hide-all" => Ok(format!("Hid {} apps", hide_all())),
        _ => Err(format!("unknown system command `{id}`")),
    }
}

/// "1.2 GB", "340 MB".
pub fn bytes(n: u64) -> String {
    let mb = n as f64 / 1_048_576.0;
    if mb >= 1024.0 {
        format!("{:.1} GB", mb / 1024.0)
    } else {
        format!("{:.0} MB", mb)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_without_changing_anything() {
        assert!(dark_mode().is_some(), "SkyLight appearance symbol is present");
        let apps = running_apps();
        assert!(apps.iter().all(|a| a.pid > 0));
        if let Some(a) = apps.first() {
            assert!(a.memory > 0, "{} has a footprint", a.name);
        }
        assert_eq!(bytes(350 * 1_048_576), "350 MB");
        assert_eq!(bytes(3 * 1_073_741_824 / 2), "1.5 GB");
        if let Ok((v, _)) = volume() {
            assert!((0.0..=100.0).contains(&v));
        }
        assert!(symbol("/System/Library/PrivateFrameworks/login.framework/Versions/Current/login", "SACLockScreenImmediate").is_some());
    }

    /// Starts Chess in the background and quits only that process. Skipped when Chess is already
    /// open. Looks the process up with pgrep: without a running run loop, NSWorkspace's app list in
    /// a test process never updates. Hide and unhide need a GUI app, so they are checked in Moo.
    #[test]
    #[ignore]
    fn acts_on_an_app_it_started() {
        let chess = || {
            let out = std::process::Command::new("/usr/bin/pgrep").args(["-x", "Chess"]).output().unwrap();
            String::from_utf8_lossy(&out.stdout).lines().next().and_then(|l| l.trim().parse::<i32>().ok())
        };
        if chess().is_some() {
            return;
        }
        assert!(std::process::Command::new("/usr/bin/open").args(["-g", "-b", "com.apple.Chess"]).status().unwrap().success());
        let mut app = None;
        for _ in 0..100 {
            app = chess();
            if app.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        let pid = app.expect("Chess started");
        std::thread::sleep(std::time::Duration::from_millis(500));
        assert!(app_action(pid, "fly").is_err());
        assert_eq!(app_action(pid, "quit").unwrap(), "Asked Chess to quit");
        for _ in 0..100 {
            if chess().is_none() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        let _ = app_action(pid, "force-quit");
        panic!("Chess did not quit");
    }
}
