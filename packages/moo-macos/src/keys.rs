//! Hotkey specs: `cmd+shift+g`, `ctrl+alt+f5`, `cmd+space`. Modifiers are `cmd`, `ctrl`, `alt`
//! (`opt`, `option`) and `shift`; the key is a name from [`KEYS`] or an alias. Key codes are
//! macOS virtual key codes, which name physical key positions (ANSI layout), as Carbon hotkeys use.

use crate::keymap::{CMD, CTRL, OPT, SHIFT};

/// Canonical key names and virtual key codes.
pub const KEYS: &[(&str, u32)] = &[
    ("a", 0), ("s", 1), ("d", 2), ("f", 3), ("h", 4), ("g", 5), ("z", 6), ("x", 7), ("c", 8),
    ("v", 9), ("b", 11), ("q", 12), ("w", 13), ("e", 14), ("r", 15), ("y", 16), ("t", 17),
    ("1", 18), ("2", 19), ("3", 20), ("4", 21), ("6", 22), ("5", 23), ("=", 24), ("9", 25),
    ("7", 26), ("-", 27), ("8", 28), ("0", 29), ("]", 30), ("o", 31), ("u", 32), ("[", 33),
    ("i", 34), ("p", 35), ("return", 36), ("l", 37), ("j", 38), ("'", 39), ("k", 40), (";", 41),
    ("\\", 42), (",", 43), ("/", 44), ("n", 45), ("m", 46), (".", 47), ("tab", 48), ("space", 49),
    ("`", 50), ("delete", 51), ("escape", 53), ("f17", 64), ("f18", 79), ("f19", 80), ("f20", 90),
    ("f5", 96), ("f6", 97), ("f7", 98), ("f3", 99), ("f8", 100), ("f9", 101), ("f11", 103),
    ("f13", 105), ("f16", 106), ("f14", 107), ("f10", 109), ("f12", 111), ("f15", 113),
    ("help", 114), ("home", 115), ("pageup", 116), ("forwarddelete", 117), ("f4", 118), ("end", 119),
    ("f2", 120), ("pagedown", 121), ("f1", 122), ("left", 123), ("right", 124), ("down", 125),
    ("up", 126),
];

const ALIASES: &[(&str, &str)] = &[
    ("enter", "return"), ("esc", "escape"), ("backspace", "delete"), ("del", "forwarddelete"),
    ("minus", "-"), ("equal", "="), ("equals", "="), ("comma", ","), ("period", "."), ("dot", "."),
    ("slash", "/"), ("backslash", "\\"), ("semicolon", ";"), ("quote", "'"), ("backtick", "`"),
    ("grave", "`"), ("leftbracket", "["), ("rightbracket", "]"), ("pgup", "pageup"),
    ("pgdn", "pagedown"), ("arrowleft", "left"), ("arrowright", "right"), ("arrowup", "up"),
    ("arrowdown", "down"),
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spec {
    /// Carbon modifier bits (`keymap::CMD` ...), as printed on the keys.
    pub mods: u32,
    pub code: u32,
    /// Canonical key name from [`KEYS`].
    pub key: &'static str,
}

fn canonical(name: &str) -> Option<(&'static str, u32)> {
    let name = ALIASES.iter().find(|(a, _)| *a == name).map_or(name, |(_, k)| k);
    KEYS.iter().find(|(k, _)| *k == name).copied()
}

pub fn is_function_key(key: &str) -> bool {
    key.len() >= 2 && key.starts_with('f') && key[1..].chars().all(|c| c.is_ascii_digit())
}

/// Parse a spec. A hotkey needs Command, Control or Option, except function keys, which may
/// stand alone or with Shift; Shift plus a letter alone would take over typing.
pub fn parse(spec: &str) -> Result<Spec, String> {
    let spec = spec.trim().to_ascii_lowercase();
    if spec.is_empty() {
        return Err("empty hotkey".into());
    }
    let mut mods = 0u32;
    let mut key = None;
    // "cmd++" is not supported; the key after the last `+` is the key ("cmd+-" works).
    for part in spec.split('+').map(str::trim) {
        let bit = match part {
            "cmd" | "command" | "⌘" => CMD,
            "ctrl" | "control" | "⌃" => CTRL,
            "alt" | "opt" | "option" | "⌥" => OPT,
            "shift" | "⇧" => SHIFT,
            _ => 0,
        };
        if bit != 0 {
            mods |= bit;
            continue;
        }
        if key.is_some() {
            return Err(format!("hotkey `{spec}` has more than one key"));
        }
        key = Some(canonical(part).ok_or_else(|| format!("unknown key `{part}` in `{spec}`"))?);
    }
    let (key, code) = key.ok_or_else(|| format!("hotkey `{spec}` has no key"))?;
    if mods & (CMD | CTRL | OPT) == 0 && !is_function_key(key) {
        return Err(format!("{} needs ⌘, ⌃ or ⌥ (only F keys work alone)", display(mods, key)));
    }
    Ok(Spec { mods, code, key })
}

/// The canonical spec string: `ctrl+alt+shift+cmd+k`.
pub fn spec_string(mods: u32, key: &str) -> String {
    crate::keymap::spec_name(mods, key)
}

/// A pressed key as a spec, if the key has a name.
pub fn from_event(code: u32, mods: u32) -> Option<String> {
    KEYS.iter().find(|(_, c)| *c == code).map(|(k, _)| spec_string(mods, k))
}

/// As printed on a Mac menu: `⌃⌥⇧⌘K`, `⌘Space`, `⌥↩`, `F5`.
pub fn display(mods: u32, key: &str) -> String {
    let mut s = String::new();
    for (bit, sym) in [(CTRL, "⌃"), (OPT, "⌥"), (SHIFT, "⇧"), (CMD, "⌘")] {
        if mods & bit != 0 {
            s.push_str(sym);
        }
    }
    let k = match key {
        "return" => "↩".to_string(),
        "delete" => "⌫".to_string(),
        "forwarddelete" => "⌦".to_string(),
        "escape" => "⎋".to_string(),
        "tab" => "⇥".to_string(),
        "left" => "←".to_string(),
        "right" => "→".to_string(),
        "up" => "↑".to_string(),
        "down" => "↓".to_string(),
        "pageup" => "⇞".to_string(),
        "pagedown" => "⇟".to_string(),
        "home" => "↖".to_string(),
        "end" => "↘".to_string(),
        "space" => "Space".to_string(),
        "help" => "Help".to_string(),
        k => k.to_ascii_uppercase(),
    };
    s.push_str(&k);
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_modifiers_keys_and_aliases() {
        assert_eq!(parse("cmd+space").unwrap(), Spec { mods: CMD, code: 49, key: "space" });
        assert_eq!(parse("Ctrl+Alt+Shift+K").unwrap(), Spec { mods: CTRL | OPT | SHIFT, code: 40, key: "k" });
        assert_eq!(parse("option+enter").unwrap().key, "return");
        assert_eq!(parse("cmd+-").unwrap().code, 27);
        assert_eq!(parse("cmd+slash").unwrap().key, "/");
        assert_eq!(parse("f5").unwrap(), Spec { mods: 0, code: 96, key: "f5" });
        assert_eq!(parse("shift+f19").unwrap().code, 80);
    }

    #[test]
    fn rejects_bad_specs() {
        assert!(parse("").is_err());
        assert!(parse("cmd").unwrap_err().contains("no key"));
        assert!(parse("cmd+a+b").unwrap_err().contains("more than one key"));
        assert!(parse("cmd+nope").unwrap_err().contains("unknown key"));
        assert!(parse("g").unwrap_err().contains("needs"));
        assert!(parse("shift+g").unwrap_err().contains("needs"));
    }

    #[test]
    fn every_key_round_trips() {
        for (k, code) in KEYS {
            let spec = from_event(*code, CMD | SHIFT).unwrap();
            let p = parse(&spec).unwrap();
            assert_eq!((p.code, p.mods, p.key), (*code, CMD | SHIFT, *k), "{spec}");
        }
        let mut codes: Vec<u32> = KEYS.iter().map(|(_, c)| *c).collect();
        codes.sort();
        codes.dedup();
        assert_eq!(codes.len(), KEYS.len(), "duplicate key codes");
    }

    #[test]
    fn displays_like_menus() {
        assert_eq!(display(CMD, "space"), "⌘Space");
        assert_eq!(display(CTRL | OPT | SHIFT | CMD, "k"), "⌃⌥⇧⌘K");
        assert_eq!(display(OPT, "return"), "⌥↩");
        assert_eq!(display(0, "f5"), "F5");
    }
}
