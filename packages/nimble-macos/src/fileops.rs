//! File actions: move to the Trash, the apps that can open a file, open it with one of them.

use std::path::Path;

use objc2::rc::Retained;
use objc2_app_kit::{NSWorkspace, NSWorkspaceOpenConfiguration};
use objc2_foundation::{NSArray, NSFileManager, NSString, NSURL};

/// Move `path` to the Trash. Returns where it went. With `NIMBLE_SYSTEM_DRY_RUN` set nothing moves.
pub fn trash(path: &str) -> Result<String, String> {
    if !Path::new(path).exists() {
        return Err(format!("no such file: {path}"));
    }
    if std::env::var_os("NIMBLE_SYSTEM_DRY_RUN").is_some() {
        return Ok(format!("dry run: trash {path}"));
    }
    let url = NSURL::fileURLWithPath(&NSString::from_str(path));
    let mut out: Option<Retained<NSURL>> = None;
    NSFileManager::defaultManager()
        .trashItemAtURL_resultingItemURL_error(&url, Some(&mut out))
        .map_err(|e| e.localizedDescription().to_string())?;
    Ok(out.and_then(|u| u.path()).map(|p| p.to_string()).unwrap_or_default())
}

pub struct OpenWith {
    pub name: String,
    pub path: String,
    pub default: bool,
}

/// Apps that can open `path`: the default one first, then the rest by name.
pub fn apps_for(path: &str) -> Vec<OpenWith> {
    let ws = NSWorkspace::sharedWorkspace();
    let url = NSURL::fileURLWithPath(&NSString::from_str(path));
    let fm = NSFileManager::defaultManager();
    let default = ws.URLForApplicationToOpenURL(&url).and_then(|u| u.path()).map(|p| p.to_string()).unwrap_or_default();
    let mut out: Vec<OpenWith> = Vec::new();
    for app in ws.URLsForApplicationsToOpenURL(&url).iter() {
        let Some(p) = app.path().map(|p| p.to_string()) else { continue };
        if out.iter().any(|o| o.path == p) {
            continue;
        }
        let name = fm.displayNameAtPath(&NSString::from_str(&p)).to_string();
        let name = name.strip_suffix(".app").unwrap_or(&name).to_string();
        out.push(OpenWith { default: p == default, name, path: p });
    }
    out.sort_by(|a, b| b.default.cmp(&a.default).then(a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    out
}

/// Open `path` with the app at `app`.
pub fn open_with(path: &str, app: &str) -> Result<(), String> {
    if !Path::new(path).exists() {
        return Err(format!("no such file: {path}"));
    }
    if !Path::new(app).exists() {
        return Err(format!("no such app: {app}"));
    }
    let file = NSURL::fileURLWithPath(&NSString::from_str(path));
    let app_url = NSURL::fileURLWithPath(&NSString::from_str(app));
    NSWorkspace::sharedWorkspace().openURLs_withApplicationAtURL_configuration_completionHandler(
        &NSArray::from_retained_slice(&[file]),
        &app_url,
        &NSWorkspaceOpenConfiguration::new(),
        None,
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/fileops-test");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join(name);
        std::fs::write(&file, "nimble").unwrap();
        file
    }

    #[test]
    fn text_files_open_with_textedit() {
        let file = scratch("open-with.txt");
        let apps = apps_for(&file.to_string_lossy());
        std::fs::remove_file(&file).unwrap();
        assert!(apps.iter().any(|a| a.path == "/System/Applications/TextEdit.app" && a.name == "TextEdit"), "{:?}", apps.iter().map(|a| &a.path).collect::<Vec<_>>());
        assert_eq!(apps.iter().filter(|a| a.default).count(), 1);
        assert!(apps[0].default);
        assert!(open_with("/no/such/file", "/System/Applications/TextEdit.app").is_err());
    }

    /// Moves a scratch file to the Trash, then deletes it from there.
    #[test]
    fn trash_moves_a_file_and_reports_where() {
        if std::env::var_os("NIMBLE_SYSTEM_DRY_RUN").is_some() {
            return;
        }
        let file = scratch("trash-me.txt");
        let went = trash(&file.to_string_lossy()).unwrap();
        assert!(!file.exists());
        assert!(went.contains("/.Trash/") && went.ends_with(".txt"), "{went}");
        std::fs::remove_file(&went).unwrap();
        assert!(trash("/no/such/file").is_err());
    }
}
