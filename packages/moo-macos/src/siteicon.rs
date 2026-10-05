//! Web images for rows: a site's `/favicon.ico` (cached as `~/Library/Caches/Moo/icons/<host>.ico`)
//! or any image URL (`<hash>.img`), refetched in the background when over a week old and registered
//! as a named image so `<image src={name}>` finds it.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use objc2::AllocAnyThread;
use objc2_app_kit::NSImage;
use objc2_foundation::{NSData, NSString, NSURL};

use crate::bridge;

const MAX_AGE: Duration = Duration::from_secs(7 * 24 * 3600);
// Team images are uploaded photos, often over a megabyte; anything larger is not an icon.
const MAX_BYTES: usize = 4 * 1024 * 1024;

static FETCHING: Mutex<Option<HashSet<String>>> = Mutex::new(None);

/// `https://example.com/any/path` -> (`https://example.com`, `example.com`). Only http(s) hosts.
fn split(url: &str) -> Option<(String, String)> {
    let (scheme, rest) = url.split_once("://")?;
    if scheme != "https" && scheme != "http" {
        return None;
    }
    let authority = rest.split(['/', '?', '#']).next()?.to_lowercase();
    let host = authority.split(':').next()?.to_string();
    let valid = host.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
    if !valid || host.starts_with('.') || host.ends_with('.') || host.contains("..") {
        return None;
    }
    Some((format!("{scheme}://{authority}"), host))
}

fn dir() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("MOO_ICONS") {
        return Some(PathBuf::from(p));
    }
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Caches/Moo/icons"))
}

fn stale(path: &Path) -> bool {
    let modified = std::fs::metadata(path).and_then(|m| m.modified());
    modified.map_or(true, |t| SystemTime::now().duration_since(t).map_or(false, |age| age > MAX_AGE))
}

/// Make the image in `path` the one named `name`. False when it isn't a readable image.
fn register(name: &str, path: &Path) -> bool {
    let file = NSString::from_str(&path.to_string_lossy());
    let Some(img) = NSImage::initWithContentsOfFile(NSImage::alloc(), &file) else { return false };
    if !img.isValid() {
        return false;
    }
    let ns_name = NSString::from_str(name);
    if let Some(old) = NSImage::imageNamed(&ns_name) {
        old.setName(None);
    }
    img.setName(Some(&ns_name))
}

fn download(source: &str) -> Option<Vec<u8>> {
    let url = NSURL::URLWithString(&NSString::from_str(source))?;
    let bytes = NSData::dataWithContentsOfURL(&url)?.to_vec();
    (!bytes.is_empty() && bytes.len() <= MAX_BYTES).then_some(bytes)
}

/// Main thread. The image name for `url`'s site icon, or "" until one is cached. When the cache is
/// missing or old a fetch starts, and callback `cb` gets the name once a new icon is registered.
pub fn site_icon(url: &str, cb: u64) -> String {
    let (Some((origin, host)), Some(dir)) = (split(url), dir()) else {
        bridge::release(cb);
        return String::new();
    };
    let path = dir.join(format!("{host}.ico"));
    cached(format!("moo-site-{host}"), dir, path, format!("{origin}/favicon.ico"), cb)
}

/// FNV-1a, so a URL gets a short stable file name.
fn fnv(s: &str) -> u64 {
    s.bytes().fold(0xcbf29ce484222325, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3))
}

/// Main thread. Like `site_icon`, for the image at `url` itself (an organization's icon, say).
pub fn image_icon(url: &str, cb: u64) -> String {
    let (Some(_), Some(dir)) = (split(url), dir()) else {
        bridge::release(cb);
        return String::new();
    };
    let key = format!("{:016x}", fnv(url));
    let path = dir.join(format!("{key}.img"));
    cached(format!("moo-image-{key}"), dir, path, url.to_string(), cb)
}

/// Main thread. The image name for the image file at `path` on disk, or "" when it isn't one. A
/// `template` image is drawn in its view's tint, like an SF Symbol.
pub fn image_file(path: &str, template: bool) -> String {
    let name = format!("moo-file-{:016x}{}", fnv(path), if template { "-t" } else { "" });
    let ns_name = NSString::from_str(&name);
    if NSImage::imageNamed(&ns_name).is_none() && !register(&name, Path::new(path)) {
        return String::new();
    }
    if let Some(img) = NSImage::imageNamed(&ns_name) {
        img.setTemplate(template);
    }
    name
}

fn cached(name: String, dir: PathBuf, path: PathBuf, source: String, cb: u64) -> String {
    let have = NSImage::imageNamed(&NSString::from_str(&name)).is_some() || register(&name, &path);
    let started = stale(&path) && FETCHING.lock().map(|mut f| f.get_or_insert_with(HashSet::new).insert(name.clone())).unwrap_or(false);
    if !started {
        bridge::release(cb);
        return if have { name } else { String::new() };
    }
    let now = if have { name.clone() } else { String::new() };
    std::thread::spawn(move || {
        let saved = download(&source).is_some_and(|bytes| {
            let tmp = path.with_extension("tmp");
            std::fs::create_dir_all(&dir).is_ok() && std::fs::write(&tmp, &bytes).is_ok() && std::fs::rename(&tmp, &path).is_ok()
        });
        bridge::on_main(move || {
            if let Ok(mut f) = FETCHING.lock() {
                f.get_or_insert_with(HashSet::new).remove(&name);
            }
            if saved && register(&name, &path) {
                bridge::call(cb, tishlang_core::Value::String(name.as_str().into()), true);
            } else {
                bridge::release(cb);
            }
        });
    });
    now
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_origin_and_host() {
        assert_eq!(split("https://hypery.ai"), Some(("https://hypery.ai".into(), "hypery.ai".into())));
        assert_eq!(split("https://Hypery.ai/settings/x?y#z"), Some(("https://hypery.ai".into(), "hypery.ai".into())));
        assert_eq!(split("http://127.0.0.1:18765/v1"), Some(("http://127.0.0.1:18765".into(), "127.0.0.1".into())));
        assert_eq!(split("file:///etc/passwd"), None);
        assert_eq!(split("https://../x"), None);
        assert_eq!(split("https://a b.com"), None);
    }
}
