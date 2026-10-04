//! Secrets (AI API keys, OAuth tokens) in the login Keychain as generic passwords under the service
//! "Nimble". A plain index file lists which accounts have one, so checking never touches the
//! Keychain (a rebuilt, differently signed binary makes macOS ask before reading).

use std::ffi::c_void;
use std::path::PathBuf;

use objc2::rc::Retained;
use objc2_foundation::NSString;

type CFTypeRef = *const c_void;

#[repr(C)]
struct Callbacks {
    _private: [u8; 0],
}

#[link(name = "Security", kind = "framework")]
extern "C" {
    static kSecClass: CFTypeRef;
    static kSecClassGenericPassword: CFTypeRef;
    static kSecAttrService: CFTypeRef;
    static kSecAttrAccount: CFTypeRef;
    static kSecValueData: CFTypeRef;
    static kSecReturnData: CFTypeRef;
    static kSecMatchLimit: CFTypeRef;
    static kSecMatchLimitOne: CFTypeRef;
    fn SecItemAdd(attrs: CFTypeRef, result: *mut CFTypeRef) -> i32;
    fn SecItemCopyMatching(query: CFTypeRef, result: *mut CFTypeRef) -> i32;
    fn SecItemUpdate(query: CFTypeRef, attrs: CFTypeRef) -> i32;
    fn SecItemDelete(query: CFTypeRef) -> i32;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    static kCFBooleanTrue: CFTypeRef;
    static kCFTypeDictionaryKeyCallBacks: Callbacks;
    static kCFTypeDictionaryValueCallBacks: Callbacks;
    fn CFDictionaryCreate(alloc: CFTypeRef, keys: *const CFTypeRef, values: *const CFTypeRef, n: isize, k: *const Callbacks, v: *const Callbacks) -> CFTypeRef;
    fn CFDataCreate(alloc: CFTypeRef, bytes: *const u8, len: isize) -> CFTypeRef;
    fn CFDataGetLength(d: CFTypeRef) -> isize;
    fn CFDataGetBytePtr(d: CFTypeRef) -> *const u8;
    fn CFRelease(cf: CFTypeRef);
}

const ERR_DUPLICATE: i32 = -25299;
const ERR_NOT_FOUND: i32 = -25300;

fn service() -> String {
    std::env::var("NIMBLE_KEYCHAIN_SERVICE").unwrap_or_else(|_| "Nimble".into())
}

unsafe fn dict(pairs: &[(CFTypeRef, CFTypeRef)]) -> CFTypeRef {
    let keys: Vec<CFTypeRef> = pairs.iter().map(|p| p.0).collect();
    let values: Vec<CFTypeRef> = pairs.iter().map(|p| p.1).collect();
    CFDictionaryCreate(
        std::ptr::null(),
        keys.as_ptr(),
        values.as_ptr(),
        pairs.len() as isize,
        &kCFTypeDictionaryKeyCallBacks,
        &kCFTypeDictionaryValueCallBacks,
    )
}

fn cf(s: &Retained<NSString>) -> CFTypeRef {
    Retained::as_ptr(s) as CFTypeRef
}

pub fn get(account: &str) -> Option<String> {
    let (svc, acct) = (NSString::from_str(&service()), NSString::from_str(account));
    unsafe {
        let q = dict(&[
            (kSecClass, kSecClassGenericPassword),
            (kSecAttrService, cf(&svc)),
            (kSecAttrAccount, cf(&acct)),
            (kSecReturnData, kCFBooleanTrue),
            (kSecMatchLimit, kSecMatchLimitOne),
        ]);
        let mut out: CFTypeRef = std::ptr::null();
        let st = SecItemCopyMatching(q, &mut out);
        CFRelease(q);
        if st != 0 || out.is_null() {
            return None;
        }
        let bytes = std::slice::from_raw_parts(CFDataGetBytePtr(out), CFDataGetLength(out) as usize).to_vec();
        CFRelease(out);
        String::from_utf8(bytes).ok()
    }
}

pub fn set(account: &str, secret: &str) -> Result<(), String> {
    let (svc, acct) = (NSString::from_str(&service()), NSString::from_str(account));
    let st = unsafe {
        let data = CFDataCreate(std::ptr::null(), secret.as_ptr(), secret.len() as isize);
        let add = dict(&[
            (kSecClass, kSecClassGenericPassword),
            (kSecAttrService, cf(&svc)),
            (kSecAttrAccount, cf(&acct)),
            (kSecValueData, data),
        ]);
        let mut st = SecItemAdd(add, std::ptr::null_mut());
        if st == ERR_DUPLICATE {
            let q = dict(&[(kSecClass, kSecClassGenericPassword), (kSecAttrService, cf(&svc)), (kSecAttrAccount, cf(&acct))]);
            let upd = dict(&[(kSecValueData, data)]);
            st = SecItemUpdate(q, upd);
            CFRelease(q);
            CFRelease(upd);
        }
        CFRelease(add);
        CFRelease(data);
        st
    };
    if st != 0 {
        return Err(format!("Keychain error {st}"));
    }
    index_edit(account, true);
    Ok(())
}

pub fn delete(account: &str) -> bool {
    let (svc, acct) = (NSString::from_str(&service()), NSString::from_str(account));
    let st = unsafe {
        let q = dict(&[(kSecClass, kSecClassGenericPassword), (kSecAttrService, cf(&svc)), (kSecAttrAccount, cf(&acct))]);
        let st = SecItemDelete(q);
        CFRelease(q);
        st
    };
    index_edit(account, false);
    st == 0 || st == ERR_NOT_FOUND
}

fn index_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("NIMBLE_KEYCHAIN_INDEX") {
        return Some(PathBuf::from(p));
    }
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support/Nimble/keychain-index.txt"))
}

fn index() -> Vec<String> {
    index_path().and_then(|p| std::fs::read_to_string(p).ok()).map(|s| s.lines().map(str::to_string).collect()).unwrap_or_default()
}

fn index_edit(account: &str, present: bool) {
    let Some(path) = index_path() else { return };
    let mut list: Vec<String> = index().into_iter().filter(|a| a != account).collect();
    if present {
        list.push(account.to_string());
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, list.join("\n") + "\n");
}

/// Whether `account` has a secret, from the index alone.
pub fn has(account: &str) -> bool {
    index().iter().any(|a| a == account)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_in_the_login_keychain() {
        std::env::set_var("NIMBLE_KEYCHAIN_SERVICE", "Nimble Test");
        let idx = std::env::temp_dir().join(format!("nimble-keychain-index-{}", std::process::id()));
        std::env::set_var("NIMBLE_KEYCHAIN_INDEX", &idx);
        let account = format!("test-{}", std::process::id());
        assert!(!has(&account));
        set(&account, "sk-first").unwrap();
        assert!(has(&account));
        assert_eq!(get(&account).as_deref(), Some("sk-first"));
        set(&account, "sk-second ✓").unwrap();
        assert_eq!(get(&account).as_deref(), Some("sk-second ✓"), "set replaces");
        assert!(delete(&account));
        assert_eq!(get(&account), None);
        assert!(!has(&account));
        let _ = std::fs::remove_file(idx);
    }
}
