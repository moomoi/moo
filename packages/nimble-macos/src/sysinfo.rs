//! Facts about this Mac for the assistant: running apps, OS, hardware, disk and battery.
//! Plain data only; wording and formatting are the Tish side's.

use std::ffi::{c_char, c_void, CStr, CString};

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::msg_send;
use objc2_app_kit::{NSApplicationActivationPolicy, NSWorkspace};
use objc2_foundation::{NSArray, NSProcessInfo, NSString, NSURL};

type CFTypeRef = *const c_void;

#[link(name = "IOKit", kind = "framework")]
extern "C" {
    fn IOPSCopyPowerSourcesInfo() -> CFTypeRef;
    fn IOPSCopyPowerSourcesList(blob: CFTypeRef) -> CFTypeRef;
    fn IOPSGetPowerSourceDescription(blob: CFTypeRef, source: CFTypeRef) -> CFTypeRef;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(cf: CFTypeRef);
}

extern "C" {
    fn sysctlbyname(name: *const c_char, old: *mut c_void, oldlen: *mut usize, new: *const c_void, newlen: usize) -> i32;
}

pub struct RunningApp {
    pub name: String,
    pub path: String,
    pub pid: i32,
    pub active: bool,
    pub hidden: bool,
}

/// Apps with a Dock icon (regular activation policy), in launch order.
pub fn running_apps() -> Vec<RunningApp> {
    NSWorkspace::sharedWorkspace()
        .runningApplications()
        .iter()
        .filter(|a| a.activationPolicy() == NSApplicationActivationPolicy::Regular)
        .map(|a| RunningApp {
            name: a.localizedName().map(|s| s.to_string()).unwrap_or_default(),
            path: a.bundleURL().and_then(|u| u.path()).map(|s| s.to_string()).unwrap_or_default(),
            pid: a.processIdentifier(),
            active: a.isActive(),
            hidden: a.isHidden(),
        })
        .collect()
}

#[derive(Default)]
pub struct Battery {
    pub percent: f64,
    pub charging: bool,
    pub on_ac: bool,
    /// Minutes; -1 while macOS is still estimating, or not applicable.
    pub minutes_to_empty: f64,
    pub minutes_to_full: f64,
}

pub struct Info {
    pub os: String,
    pub model: String,
    pub chip: String,
    pub cores: usize,
    pub memory_bytes: u64,
    pub uptime_secs: f64,
    pub disk_total: u64,
    /// What Finder shows as available (includes purgeable space).
    pub disk_free: u64,
    pub battery: Option<Battery>,
}

fn sysctl_string(name: &str) -> String {
    let Ok(c) = CString::new(name) else { return String::new() };
    let mut len = 0usize;
    unsafe {
        if sysctlbyname(c.as_ptr(), std::ptr::null_mut(), &mut len, std::ptr::null(), 0) != 0 || len == 0 {
            return String::new();
        }
        let mut buf = vec![0u8; len];
        if sysctlbyname(c.as_ptr(), buf.as_mut_ptr() as *mut c_void, &mut len, std::ptr::null(), 0) != 0 {
            return String::new();
        }
        CStr::from_bytes_until_nul(&buf).map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
    }
}

unsafe fn number(dict: &AnyObject, key: &str) -> Option<f64> {
    let k = NSString::from_str(key);
    let v: *mut AnyObject = msg_send![dict, objectForKey: &*k];
    if v.is_null() {
        return None;
    }
    let n: f64 = msg_send![&*v, doubleValue];
    Some(n)
}

unsafe fn string(dict: &AnyObject, key: &str) -> String {
    let k = NSString::from_str(key);
    let v: *mut AnyObject = msg_send![dict, objectForKey: &*k];
    if v.is_null() {
        return String::new();
    }
    let s: *mut NSString = msg_send![&*v, description];
    if s.is_null() { String::new() } else { (*s).to_string() }
}

fn volume() -> (u64, u64) {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
    let url = NSURL::fileURLWithPath(&NSString::from_str(&home));
    let keys = ["NSURLVolumeTotalCapacityKey", "NSURLVolumeAvailableCapacityForImportantUsageKey"];
    let names: Vec<Retained<NSString>> = keys.iter().map(|k| NSString::from_str(k)).collect();
    let list = NSArray::from_retained_slice(&names);
    unsafe {
        let dict: *mut AnyObject = msg_send![&*url, resourceValuesForKeys: &*list, error: std::ptr::null_mut::<*mut AnyObject>()];
        if dict.is_null() {
            return (0, 0);
        }
        let total = number(&*dict, keys[0]).unwrap_or(0.0);
        let free = number(&*dict, keys[1]).unwrap_or(0.0);
        (total.max(0.0) as u64, free.max(0.0) as u64)
    }
}

fn battery() -> Option<Battery> {
    unsafe {
        let blob = IOPSCopyPowerSourcesInfo();
        if blob.is_null() {
            return None;
        }
        let list = IOPSCopyPowerSourcesList(blob);
        let mut found = None;
        if !list.is_null() {
            let sources = &*(list as *const NSArray<AnyObject>);
            for s in sources.iter() {
                let d = IOPSGetPowerSourceDescription(blob, Retained::as_ptr(&s) as CFTypeRef);
                if d.is_null() {
                    continue;
                }
                let d = &*(d as *const AnyObject);
                if string(d, "Type") != "InternalBattery" {
                    continue;
                }
                let cur = number(d, "Current Capacity").unwrap_or(0.0);
                let max = number(d, "Max Capacity").unwrap_or(100.0).max(1.0);
                found = Some(Battery {
                    percent: (cur / max * 100.0).round(),
                    charging: number(d, "Is Charging").unwrap_or(0.0) != 0.0,
                    on_ac: string(d, "Power Source State") == "AC Power",
                    minutes_to_empty: number(d, "Time to Empty").unwrap_or(-1.0),
                    minutes_to_full: number(d, "Time to Full Charge").unwrap_or(-1.0),
                });
                break;
            }
            CFRelease(list);
        }
        CFRelease(blob);
        found
    }
}

pub fn info() -> Info {
    let p = NSProcessInfo::processInfo();
    let (disk_total, disk_free) = volume();
    Info {
        os: p.operatingSystemVersionString().to_string(),
        model: sysctl_string("hw.model"),
        chip: sysctl_string("machdep.cpu.brand_string"),
        cores: p.activeProcessorCount(),
        memory_bytes: p.physicalMemory(),
        uptime_secs: p.systemUptime(),
        disk_total,
        disk_free,
        battery: battery(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_this_mac() {
        let i = info();
        assert!(i.os.contains("Version"), "{}", i.os);
        assert!(!i.model.is_empty());
        assert!(i.memory_bytes > 0 && i.disk_total > i.disk_free && i.disk_free > 0);
        assert!(i.uptime_secs > 0.0);
    }
}
