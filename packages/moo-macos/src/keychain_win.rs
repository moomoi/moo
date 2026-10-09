//! Windows: secrets (AI API keys, plugin tokens) in Credential Manager as generic credentials
//! named `Moo:<account>`, the names the Tish side (natives.windows.tish) uses too. Same API as
//! keychain.rs.

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Security::Credentials::*;

fn target(account: &str) -> Vec<u16> {
    let service = std::env::var("MOO_KEYCHAIN_SERVICE").unwrap_or_else(|_| "Moo".into());
    format!("{service}:{account}").encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn get(account: &str) -> Option<String> {
    let t = target(account);
    unsafe {
        let mut p: *mut CREDENTIALW = std::ptr::null_mut();
        CredReadW(PCWSTR(t.as_ptr()), CRED_TYPE_GENERIC, None, &mut p).ok()?;
        let c = &*p;
        let secret = String::from_utf8_lossy(std::slice::from_raw_parts(c.CredentialBlob, c.CredentialBlobSize as usize)).into_owned();
        CredFree(p as *const _);
        Some(secret)
    }
}

pub fn set(account: &str, secret: &str) -> Result<(), String> {
    let mut t = target(account);
    let mut bytes = secret.as_bytes().to_vec();
    let cred = CREDENTIALW {
        Type: CRED_TYPE_GENERIC,
        TargetName: PWSTR(t.as_mut_ptr()),
        CredentialBlobSize: bytes.len() as u32,
        CredentialBlob: bytes.as_mut_ptr(),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        ..Default::default()
    };
    unsafe { CredWriteW(&cred, 0) }.map_err(|e| e.message())
}

pub fn delete(account: &str) -> bool {
    let t = target(account);
    unsafe { CredDeleteW(PCWSTR(t.as_ptr()), CRED_TYPE_GENERIC, None) }.is_ok()
}

pub fn has(account: &str) -> bool {
    get(account).is_some()
}
