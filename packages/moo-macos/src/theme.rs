//! The panel's look, as set from Tish with `setTheme(theme)` (see app/src/theme.tish): glass and
//! vibrancy colours, corner radius, margins, position and every animation timing. Nothing visual
//! is decided here; until Tish sets a theme the panel is plain (untinted, square, no animation).

use std::cell::RefCell;
use std::ffi::CString;

use objc2::rc::Retained;
use objc2::runtime::Sel;
use objc2::{msg_send, ClassType};
use objc2_app_kit::{NSColor, NSGlassEffectViewStyle, NSVisualEffectMaterial};
use tishlang_core::Value;

/// A colour for light and dark mode: an AppKit semantic name ("label", "controlAccent") or hex
/// "#RRGGBB" / "#RRGGBBAA". Empty means none.
#[derive(Clone, Default)]
pub struct Color {
    light: String,
    dark: String,
}

impl Color {
    pub fn resolve(&self, dark: bool) -> Option<Retained<NSColor>> {
        parse_color(if dark { &self.dark } else { &self.light })
    }
}

#[derive(Clone, Copy)]
pub struct Spring {
    /// Period of the undamped oscillation, in seconds.
    pub response: f64,
    /// Damping ratio; below 1 overshoots.
    pub damping: f64,
    /// Length of the animation; 0 turns it off.
    pub secs: f64,
}

#[derive(Clone)]
pub struct Theme {
    /// Corner radius of the full panel (the pieces of a shape carry their own).
    pub radius: f64,
    /// Clear border around the panel shape, so springs can overshoot without clipping.
    pub margin: f64,
    /// Panel top edge, as a fraction of the screen's visible height from the bottom.
    pub top: f64,
    pub glass_style: NSGlassEffectViewStyle,
    /// Glass pieces closer than this melt into one.
    pub glass_spacing: f64,
    /// Glass tint for a shape of several pieces (the idle bar) and for the one-piece panel.
    pub glass_tint_bar: Color,
    pub glass_tint_panel: Color,
    /// Before macOS 26: vibrancy material, its tint and hairline edge, and the window shadow.
    pub material: NSVisualEffectMaterial,
    pub vibrancy_tint: Color,
    pub edge: Color,
    pub edge_width: f64,
    pub shadow: bool,
    pub open: Spring,
    /// Delay between successive pieces leaving the first one.
    pub stagger: f64,
    /// The Tish view fades in from `fade_delay` over `fade_secs`.
    pub fade_delay: f64,
    pub fade_secs: f64,
    /// Starting size of the first piece, as a fraction of its final width and height.
    pub open_scale_bar: (f64, f64),
    pub open_scale_panel: (f64, f64),
    /// How far the other pieces start tucked inside the first one.
    pub tuck: f64,
    /// Opacity gained per unit of spring progress by pieces springing out (higher is sooner).
    pub piece_fade: f64,
    pub morph: Spring,
    /// Same for pieces appearing or disappearing during a morph.
    pub morph_fade: f64,
}

impl Default for Theme {
    fn default() -> Self {
        let off = Spring { response: 1.0, damping: 1.0, secs: 0.0 };
        Theme {
            radius: 0.0,
            margin: 0.0,
            top: 0.5,
            glass_style: NSGlassEffectViewStyle::Regular,
            glass_spacing: 0.0,
            glass_tint_bar: Color::default(),
            glass_tint_panel: Color::default(),
            material: NSVisualEffectMaterial::WindowBackground,
            vibrancy_tint: Color::default(),
            edge: Color::default(),
            edge_width: 0.0,
            shadow: true,
            open: off,
            stagger: 0.0,
            fade_delay: 0.0,
            fade_secs: 0.0,
            open_scale_bar: (1.0, 1.0),
            open_scale_panel: (1.0, 1.0),
            tuck: 0.0,
            piece_fade: 1.0,
            morph: off,
            morph_fade: 1.0,
        }
    }
}

thread_local! {
    static THEME: RefCell<Theme> = RefCell::new(Theme::default());
}

pub fn get() -> Theme {
    THEME.with(|t| t.borrow().clone())
}

/// Read a theme object (missing fields keep the plain defaults) and make it current.
pub fn set(v: &Value) {
    let d = Theme::default();
    let at = |path: &[&str]| path.iter().try_fold(v.clone(), |cur, k| crate::field(Some(&cur), k));
    let num = |path: &[&str], def: f64| at(path).and_then(|x| x.as_number()).unwrap_or(def);
    let text = |path: &[&str]| match at(path) {
        Some(Value::String(s)) => s.to_string(),
        _ => String::new(),
    };
    let color = |path: &[&str]| match at(path) {
        Some(Value::String(s)) => Color { light: s.to_string(), dark: s.to_string() },
        Some(c @ Value::Object(_)) => {
            let part = |k| match crate::field(Some(&c), k) {
                Some(Value::String(s)) => s.to_string(),
                _ => String::new(),
            };
            Color { light: part("light"), dark: part("dark") }
        }
        _ => Color::default(),
    };
    let pair = |path: &[&str], def: (f64, f64)| match at(path) {
        Some(Value::Array(a)) => {
            let a = a.borrow();
            let n = |i: usize| a.get(i).and_then(|x| x.as_number());
            (n(0).unwrap_or(def.0), n(1).unwrap_or(def.1))
        }
        _ => def,
    };
    let spring = |path: &str, def: Spring| Spring {
        response: num(&["motion", path, "response"], def.response),
        damping: num(&["motion", path, "damping"], def.damping),
        secs: num(&["motion", path, "secs"], def.secs),
    };
    let theme = Theme {
        radius: num(&["panel", "radius"], d.radius),
        margin: num(&["panel", "margin"], d.margin),
        top: num(&["panel", "top"], d.top),
        glass_style: if text(&["glass", "style"]) == "clear" { NSGlassEffectViewStyle::Clear } else { NSGlassEffectViewStyle::Regular },
        glass_spacing: num(&["glass", "spacing"], d.glass_spacing),
        glass_tint_bar: color(&["glass", "tint", "bar"]),
        glass_tint_panel: color(&["glass", "tint", "panel"]),
        material: material(&text(&["vibrancy", "material"])),
        vibrancy_tint: color(&["vibrancy", "tint"]),
        edge: color(&["vibrancy", "edge"]),
        edge_width: num(&["vibrancy", "edgeWidth"], d.edge_width),
        shadow: !matches!(at(&["vibrancy", "shadow"]), Some(Value::Bool(false))),
        open: spring("open", d.open),
        stagger: num(&["motion", "open", "stagger"], d.stagger),
        fade_delay: num(&["motion", "open", "fadeDelay"], d.fade_delay),
        fade_secs: num(&["motion", "open", "fadeSecs"], d.fade_secs),
        open_scale_bar: pair(&["motion", "open", "scale", "bar"], d.open_scale_bar),
        open_scale_panel: pair(&["motion", "open", "scale", "panel"], d.open_scale_panel),
        tuck: num(&["motion", "open", "tuck"], d.tuck),
        piece_fade: num(&["motion", "open", "pieceFade"], d.piece_fade),
        morph: spring("morph", d.morph),
        morph_fade: num(&["motion", "morph", "fade"], d.morph_fade),
    };
    THEME.with(|t| *t.borrow_mut() = theme);
}

fn material(name: &str) -> NSVisualEffectMaterial {
    match name {
        "titlebar" => NSVisualEffectMaterial::Titlebar,
        "selection" => NSVisualEffectMaterial::Selection,
        "menu" => NSVisualEffectMaterial::Menu,
        "popover" => NSVisualEffectMaterial::Popover,
        "sidebar" => NSVisualEffectMaterial::Sidebar,
        "headerView" => NSVisualEffectMaterial::HeaderView,
        "sheet" => NSVisualEffectMaterial::Sheet,
        "hud" => NSVisualEffectMaterial::HUDWindow,
        "fullScreenUI" => NSVisualEffectMaterial::FullScreenUI,
        "toolTip" => NSVisualEffectMaterial::ToolTip,
        "contentBackground" => NSVisualEffectMaterial::ContentBackground,
        "underWindowBackground" => NSVisualEffectMaterial::UnderWindowBackground,
        "underPageBackground" => NSVisualEffectMaterial::UnderPageBackground,
        _ => NSVisualEffectMaterial::WindowBackground,
    }
}

/// "#RRGGBB", "#RRGGBBAA", or an `NSColor` class colour by name without "Color" ("label" →
/// `labelColor`), which follows light / dark mode by itself.
pub fn parse_color(s: &str) -> Option<Retained<NSColor>> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix('#') {
        let v = u32::from_str_radix(hex, 16).ok()?;
        let (r, g, b, a) = match hex.len() {
            6 => (v >> 16, v >> 8, v, 255),
            8 => (v >> 24, v >> 16, v >> 8, v),
            _ => return None,
        };
        let c = |x: u32| (x & 255) as f64 / 255.0;
        return Some(NSColor::colorWithSRGBRed_green_blue_alpha(c(r), c(g), c(b), c(a)));
    }
    // Only real NSColor names: the theme string becomes a selector, and an arbitrary `…Color`
    // class method that isn't a colour would be undefined behaviour once typed as an NSColor.
    if !NAMED_COLORS.contains(&s) {
        return None;
    }
    let sel = Sel::register(&CString::new(format!("{s}Color")).ok()?);
    let cls = NSColor::class();
    let known: bool = unsafe { msg_send![cls, respondsToSelector: sel] };
    if !known {
        return None;
    }
    unsafe { msg_send![cls, performSelector: sel] }
}

/// NSColor class methods a theme may name (`"label"` → `labelColor`): semantic, system and basic.
const NAMED_COLORS: &[&str] = &[
    "label", "secondaryLabel", "tertiaryLabel", "quaternaryLabel", "quinaryLabel", "text", "placeholderText",
    "selectedText", "textBackground", "selectedTextBackground", "keyboardFocusIndicator", "unemphasizedSelectedText",
    "unemphasizedSelectedTextBackground", "link", "separator", "selectedContentBackground",
    "unemphasizedSelectedContentBackground", "selectedMenuItemText", "grid", "header", "headerText", "control",
    "controlBackground", "controlText", "disabledControlText", "selectedControl", "selectedControlText",
    "alternateSelectedControlText", "scrubberTexturedBackground", "windowBackground", "windowFrameText",
    "underPageBackground", "findHighlight", "highlight", "shadow", "controlAccent", "systemRed", "systemGreen",
    "systemBlue", "systemOrange", "systemYellow", "systemBrown", "systemPink", "systemPurple", "systemGray",
    "systemTeal", "systemIndigo", "systemMint", "systemCyan", "systemFill", "secondarySystemFill",
    "tertiarySystemFill", "quaternarySystemFill", "quinarySystemFill", "clear", "black", "white", "gray",
    "darkGray", "lightGray", "red", "green", "blue", "cyan", "yellow", "magenta", "orange", "purple", "brown",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_and_named_colours() {
        let c = parse_color("#FF000080").expect("hex");
        assert!((c.alphaComponent() - 128.0 / 255.0).abs() < 1e-6);
        assert!(parse_color("#12345").is_none());
        assert!(parse_color("controlAccent").is_some());
        assert!(parse_color("noSuchThing").is_none());
        assert!(parse_color("").is_none());
    }
}
