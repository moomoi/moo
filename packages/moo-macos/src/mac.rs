//! AppKit side: launcher panel chrome, show/hide, key routing, global hotkey, launch, icons.
//!
//! Tish callbacks never run inside an AppKit or Carbon handler: they are queued and flushed from a
//! main-queue block under `run_with_current_root`, because a callback that calls `setState`
//! re-commits the tree synchronously.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr::NonNull;

use block2::RcBlock;
use dispatch2::{DispatchQueue, DispatchTime};
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};
use objc2::{define_class, msg_send, sel, ClassType, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAppearanceCustomization, NSAppearanceNameAqua, NSAppearanceNameDarkAqua, NSApplication, NSApplicationActivationPolicy,
    NSAutoresizingMaskOptions, NSBackingStoreType, NSColor, NSGlassEffectContainerView, NSGlassEffectView,
    NSEvent, NSEventMask, NSEventModifierFlags, NSEventType, NSMenu, NSMenuItem,
    NSPanel, NSResponder, NSScreen, NSStatusBar, NSStatusItem, NSTextField, NSView, NSVisualEffectBlendingMode,
    NSVisualEffectState, NSVisualEffectView, NSWindow, NSWindowButton, NSWindowCollectionBehavior,
    NSWindowDidBecomeKeyNotification, NSWindowDidResignKeyNotification, NSWindowStyleMask,
    NSImage, NSPasteboard, NSPasteboardTypeString, NSWindowTitleVisibility, NSWorkspace,
};
use objc2_foundation::{
    NSArray, NSNotification, NSNotificationCenter, NSObject, NSObjectProtocol, NSPoint, NSRange, NSRect, NSSize, NSString,
    NSTimer, NSURL,
};
use tishlang_core::{Value, VmRef};

use crate::{clip, files, index, keymap, keys, theme, watch};

const ICON_SLOTS: usize = 512;
use tishlang_ui::runtime::{run_with_current_root, LEGACY_ROOT_ID};

thread_local! {
    static ON_KEY: RefCell<Option<Value>> = const { RefCell::new(None) };
    static ON_SHOW: RefCell<Option<Value>> = const { RefCell::new(None) };
    static ON_HOTKEY: RefCell<Option<Value>> = const { RefCell::new(None) };
    static START_HIDDEN: Cell<bool> = const { Cell::new(false) };
    static PENDING: RefCell<Vec<(&'static str, String)>> = const { RefCell::new(Vec::new()) };
    static MONITOR: RefCell<Option<Retained<AnyObject>>> = const { RefCell::new(None) };
    static SCROLL_MONITOR: RefCell<Option<Retained<AnyObject>>> = const { RefCell::new(None) };
    /// Scrolling not yet worth a whole row, carried into the next event.
    static SCROLL_REST: Cell<f64> = const { Cell::new(0.0) };
    static PANEL_SIZE: Cell<(f64, f64)> = const { Cell::new((720.0, 440.0)) };
    static ICONS: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
    static HOTKEY_HANDLER: Cell<bool> = const { Cell::new(false) };
    static HOTKEY_IDS: Cell<u32> = const { Cell::new(1) };
    static SHOWN: Cell<bool> = const { Cell::new(false) };
    static PANEL: RefCell<Option<Retained<NSWindow>>> = const { RefCell::new(None) };
    static ON_FILES: RefCell<Option<Value>> = const { RefCell::new(None) };
    static ICON_RING: RefCell<(usize, Vec<String>)> = const { RefCell::new((0, Vec::new())) };
    static STATUS: RefCell<Option<StatusItem>> = const { RefCell::new(None) };
    static STATUS_MENU: RefCell<Option<Box<dyn Fn(&str)>>> = const { RefCell::new(None) };
    /// Panel height and its rounded pieces; empty means one full rounded rect.
    static SHAPE: RefCell<(f64, Vec<Piece>)> = const { RefCell::new((0.0, Vec::new())) };
    /// Holds one glass (or blur) view per shape piece, behind the layout.
    static PIECES_HOST: RefCell<Option<Retained<NSView>>> = const { RefCell::new(None) };
    static PIECES: RefCell<Vec<Retained<NSView>>> = const { RefCell::new(Vec::new()) };
    /// The top-anchored view holding the root view; faded in while the panel opens.
    static CONTENT: RefCell<Option<Retained<NSView>>> = const { RefCell::new(None) };
    /// Liquid Glass (macOS 26) is available; otherwise pieces are tinted vibrancy views.
    static GLASS: Cell<bool> = const { Cell::new(false) };
    static OPEN_ANIM: RefCell<Option<Retained<NSTimer>>> = const { RefCell::new(None) };
    static MORPH_ANIM: RefCell<Option<Retained<NSTimer>>> = const { RefCell::new(None) };
}

pub fn set_callbacks(on_key: Option<Value>, on_show: Option<Value>, on_hotkey: Option<Value>) {
    ON_KEY.with(|c| *c.borrow_mut() = on_key);
    ON_SHOW.with(|c| *c.borrow_mut() = on_show);
    ON_HOTKEY.with(|c| *c.borrow_mut() = on_hotkey);
}

/// Keep the panel hidden when setup finishes (started in the background, e.g. by the CLI).
pub fn set_start_hidden(hidden: bool) {
    START_HIDDEN.with(|c| c.set(hidden));
}

pub fn set_panel_size(w: f64, h: f64) {
    PANEL_SIZE.with(|c| c.set((w, h)));
}

fn app(mtm: MainThreadMarker) -> Retained<NSApplication> {
    NSApplication::sharedApplication(mtm)
}

fn main_window(mtm: MainThreadMarker) -> Option<Retained<NSWindow>> {
    if let Some(p) = PANEL.with(|p| p.borrow().clone()) {
        return Some(p);
    }
    host_window(mtm)
}

fn host_window(mtm: MainThreadMarker) -> Option<Retained<NSWindow>> {
    let windows = app(mtm).windows();
    let n = windows.count();
    (0..n).map(|i| windows.objectAtIndex(i)).find(|w| w.canBecomeKeyWindow())
}

// ── Deferred callbacks ──────────────────────────────────────────────────────

/// Run Tish code against the launcher root. A re-render can rebuild the search field and drop its
/// focus, so focus is handed back afterwards.
pub(crate) fn with_ui<R>(f: impl FnOnce() -> R) -> R {
    let out = run_with_current_root(LEGACY_ROOT_ID, f);
    restore_focus();
    out
}

/// Queue `(callback, arg)` and flush it from the main queue once the current handler returns.
fn defer_callback(which: &'static str, arg: impl Into<String>) {
    PENDING.with(|q| q.borrow_mut().push((which, arg.into())));
    DispatchQueue::main().exec_async(flush_pending);
}

fn flush_pending() {
    let events = PENDING.with(|q| std::mem::take(&mut *q.borrow_mut()));
    if events.is_empty() {
        return;
    }
    with_ui(|| {
        for (which, arg) in events {
            let cb = match which {
                "key" => ON_KEY.with(|c| c.borrow().clone()),
                "hotkey" => ON_HOTKEY.with(|c| c.borrow().clone()),
                _ => ON_SHOW.with(|c| c.borrow().clone()),
            };
            if let Some(Value::Function(f)) = cb {
                let _ = f.call(&[Value::String(arg.as_str().into())]);
                debug_log(&format!("{which} {arg} handled"));
            }
        }
    });
}

// ── Panel chrome and visibility ─────────────────────────────────────────────

fn position_panel(w: &NSWindow, mtm: MainThreadMarker) {
    let Some(screen) = NSScreen::mainScreen(mtm) else { return };
    let vf = screen.visibleFrame();
    let f = w.frame();
    let x = vf.origin.x + (vf.size.width - f.size.width) / 2.0;
    let top = vf.origin.y + vf.size.height * theme::get().top + margin();
    w.setFrameOrigin(NSPoint::new(x, top - f.size.height));
}

/// Clear border around the panel shape (from the theme).
fn margin() -> f64 {
    theme::get().margin
}

/// Restyle the panel after `setTheme`; before setup the theme is simply used when it is built.
pub fn theme_changed() {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let Some(w) = PANEL.with(|p| p.borrow().clone()) else { return };
    let t = theme::get();
    let host = PIECES_HOST.with(|b| b.borrow().clone());
    if let Some(container) = host.and_then(|h| unsafe { h.superview() }).and_then(|s| s.downcast::<NSGlassEffectContainerView>().ok()) {
        container.setSpacing(t.glass_spacing);
    }
    if !GLASS.with(|g| g.get()) {
        w.setHasShadow(t.shadow);
    }
    // Pieces are recreated so their glass style or material is the new one.
    stop_open_animation();
    stop_morph();
    set_piece_count(0, mtm);
    apply_shape(&w, mtm);
    position_panel(&w, mtm);
}

define_class!(
    /// A borderless panel that can still take keys (AppKit refuses key status to borderless
    /// windows by default).
    #[unsafe(super(NSPanel, NSWindow, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "MooLauncherPanel"]
    struct LauncherPanel;

    impl LauncherPanel {
        #[unsafe(method(canBecomeKeyWindow))]
        fn can_become_key(&self) -> bool {
            true
        }

        #[unsafe(method(canBecomeMainWindow))]
        fn can_become_main(&self) -> bool {
            false
        }

        // Quick Look asks the key window's responder chain for a controller; the panel stays key
        // while previewing so the arrow keys keep moving through the results.
        #[unsafe(method(acceptsPreviewPanelControl:))]
        fn accepts_preview(&self, _panel: &AnyObject) -> bool {
            true
        }

        #[unsafe(method(beginPreviewPanelControl:))]
        fn begin_preview(&self, panel: &AnyObject) {
            let source = PREVIEW_SOURCE.with(|s| s.borrow().clone());
            if let Some(source) = source {
                let _: () = unsafe { msg_send![panel, setDataSource: &*source] };
            }
        }

        #[unsafe(method(endPreviewPanelControl:))]
        fn end_preview(&self, panel: &AnyObject) {
            let _: () = unsafe { msg_send![panel, setDataSource: std::ptr::null::<AnyObject>()] };
        }
    }
);

define_class!(
    /// Hands Quick Look the one file being previewed.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "MooPreviewSource"]
    struct PreviewSource;

    impl PreviewSource {
        #[unsafe(method(numberOfPreviewItemsInPreviewPanel:))]
        fn count(&self, _panel: &AnyObject) -> isize {
            PREVIEW_PATH.with(|p| !p.borrow().is_empty()) as isize
        }

        #[unsafe(method_id(previewPanel:previewItemAtIndex:))]
        fn item(&self, _panel: &AnyObject, _index: isize) -> Option<Retained<NSURL>> {
            let path = PREVIEW_PATH.with(|p| p.borrow().clone());
            (!path.is_empty()).then(|| NSURL::fileURLWithPath(&NSString::from_str(&path)))
        }
    }
);

#[link(name = "Quartz", kind = "framework")]
extern "C" {}

thread_local! {
    static PREVIEW_PATH: RefCell<String> = const { RefCell::new(String::new()) };
    static PREVIEW_SOURCE: RefCell<Option<Retained<PreviewSource>>> = const { RefCell::new(None) };
}

fn preview_panel(create: bool) -> Option<Retained<AnyObject>> {
    let cls = AnyClass::get(c"QLPreviewPanel")?;
    unsafe {
        if !create {
            let exists: bool = msg_send![cls, sharedPreviewPanelExists];
            if !exists {
                return None;
            }
        }
        msg_send![cls, sharedPreviewPanel]
    }
}

pub fn quick_look_visible() -> bool {
    preview_panel(false).is_some_and(|p| unsafe { msg_send![&*p, isVisible] })
}

/// Preview `path` in Quick Look (or switch the open preview to it); an empty path closes it.
/// Returns whether the preview is now open.
pub fn quick_look(path: &str) -> bool {
    let Some(mtm) = MainThreadMarker::new() else { return false };
    if path.is_empty() || !SHOWN.with(|s| s.get()) {
        close_quick_look();
        return false;
    }
    PREVIEW_PATH.with(|p| *p.borrow_mut() = path.to_string());
    if PREVIEW_SOURCE.with(|s| s.borrow().is_none()) {
        let source: Retained<PreviewSource> = unsafe { msg_send![PreviewSource::alloc(mtm), init] };
        PREVIEW_SOURCE.with(|s| *s.borrow_mut() = Some(source));
    }
    let Some(panel) = preview_panel(true) else { return false };
    unsafe {
        let _: () = msg_send![&*panel, updateController];
        let _: () = msg_send![&*panel, reloadData];
        let level = main_window(mtm).map_or(3, |w| w.level());
        let _: () = msg_send![&*panel, setLevel: level + 1];
        let _: () = msg_send![&*panel, orderFront: std::ptr::null::<AnyObject>()];
    }
    // Quick Look sizes and centres itself once the item has loaded, so move it afterwards.
    let when = DispatchTime::try_from(std::time::Duration::from_millis(250)).unwrap_or(DispatchTime::NOW);
    let _ = DispatchQueue::main().after(when, || {
        let (Some(mtm), Some(panel)) = (MainThreadMarker::new(), preview_panel(false)) else { return };
        if let Some(w) = main_window(mtm).filter(|_| quick_look_visible()) {
            unsafe { beside_panel(&panel, &w) };
        }
    });
    debug_log(&format!("quick look {path}: visible={}", quick_look_visible()));
    true
}

/// Quick Look centres itself, over the launcher; put it beside the launcher (right, else left)
/// when the screen has room, so the results stay in view.
unsafe fn beside_panel(preview: &AnyObject, w: &NSWindow) {
    const GAP: f64 = 12.0;
    let Some(screen) = w.screen() else { return };
    let area = screen.visibleFrame();
    let p = w.frame();
    let f: NSRect = msg_send![preview, frame];
    let room_right = area.origin.x + area.size.width - (p.origin.x + p.size.width) - 2.0 * GAP;
    let room_left = p.origin.x - area.origin.x - 2.0 * GAP;
    let (room, right) = if room_right >= room_left { (room_right, true) } else { (room_left, false) };
    if room < 320.0 {
        return;
    }
    let width = f.size.width.min(room);
    let height = f.size.height * width / f.size.width;
    let x = if right { p.origin.x + p.size.width + GAP } else { p.origin.x - GAP - width };
    let top = p.origin.y + p.size.height;
    let frame = NSRect::new(NSPoint::new(x, (top - height).max(area.origin.y)), NSSize::new(width, height));
    let _: () = msg_send![preview, setFrame: frame, display: true, animate: false];
}

pub fn close_quick_look() {
    PREVIEW_PATH.with(|p| p.borrow_mut().clear());
    if let Some(panel) = preview_panel(false) {
        let _: () = unsafe { msg_send![&*panel, orderOut: std::ptr::null::<AnyObject>()] };
    }
}

define_class!(
    /// The panel's content view. The window takes every click (see `adopt_into_panel`), so a click
    /// that nothing inside handled and that missed the glass counts as a click outside: it closes
    /// the panel, as it would if the click had gone to the window below.
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "MooPanelStage"]
    struct PanelStage;

    impl PanelStage {
        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            let p = self.convertPoint_fromView(event.locationInWindow(), None);
            let inside = |r: NSRect| {
                p.x >= r.origin.x && p.x <= r.origin.x + r.size.width && p.y >= r.origin.y && p.y <= r.origin.y + r.size.height
            };
            if !PIECES.with(|ps| ps.borrow().iter().any(|v| inside(v.frame()))) {
                hide();
            }
        }
    }
);

define_class!(
    /// Top-left origin: the layout keeps its full height, so when the panel is shorter (the idle
    /// bar) the header stays at the top and the rest is clipped below.
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "MooTopAnchoredView"]
    struct TopAnchoredView;

    impl TopAnchoredView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }
    }
);

/// macOS 14+ refuses activation requests from background apps, so the launcher must take keys
/// without activating, and only an NSPanel honours `NonactivatingPanel`. tish-macos creates a plain
/// NSWindow (re-classing it breaks AppKit's KVO), so its root view moves into our own panel. The
/// host keeps measuring its window's content view, so that window keeps a same-size placeholder.
///
/// The panel is borderless and clear, the theme's margin larger than its shape on every side. Behind the root
/// view sits one Liquid Glass view per piece of the shape, in a glass container so pieces that
/// touch melt together (before macOS 26: rounded, tinted vibrancy views).
fn adopt_into_panel(host_window: &NSWindow, mtm: MainThreadMarker) -> Option<Retained<NSWindow>> {
    let root = host_window.contentView()?;
    let (pw, ph) = PANEL_SIZE.with(|c| c.get());
    let theme = theme::get();
    let margin = theme.margin;
    let rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(pw, ph));
    let outer = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(pw + 2.0 * margin, ph + 2.0 * margin));
    let glass = std::env::var_os("MOO_NO_GLASS").is_none() && AnyClass::get(c"NSGlassEffectView").is_some();
    GLASS.with(|g| g.set(glass));

    let placeholder = NSView::initWithFrame(NSView::alloc(mtm), rect);
    host_window.setContentView(Some(&placeholder));
    host_window.setContentSize(NSSize::new(pw, ph));
    host_window.orderOut(None);

    let panel: Retained<LauncherPanel> = unsafe {
        msg_send![
            LauncherPanel::alloc(mtm),
            initWithContentRect: outer,
            styleMask: NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
            backing: NSBackingStoreType::Buffered,
            defer: false
        ]
    };
    let panel: Retained<NSPanel> = Retained::into_super(panel);
    unsafe { panel.setReleasedWhenClosed(false) };
    panel.setFloatingPanel(true);
    panel.setBecomesKeyOnlyIfNeeded(false);
    panel.setHidesOnDeactivate(false);
    panel.setAppearance(host_window.appearance().as_deref());
    panel.setOpaque(false);
    panel.setBackgroundColor(Some(&NSColor::clearColor()));
    // A clear window lets clicks on its transparent pixels through to the window below, which is
    // nearly all of it (glass is composited by the window server, rows and the field are clear):
    // the click would land elsewhere and the panel close on losing key. Setting this explicitly
    // turns that off; PanelStage treats clicks off the glass as clicks outside.
    panel.setIgnoresMouseEvents(false);
    // Glass draws its own shadow, which follows the pieces while they move.
    panel.setHasShadow(!glass && theme.shadow);

    let sizable = NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable;
    let stage: Retained<PanelStage> = unsafe { msg_send![PanelStage::alloc(mtm), initWithFrame: outer] };
    let pieces = NSView::initWithFrame(NSView::alloc(mtm), outer);
    pieces.setAutoresizingMask(sizable);
    if glass {
        let container = NSGlassEffectContainerView::initWithFrame(NSGlassEffectContainerView::alloc(mtm), outer);
        container.setSpacing(theme.glass_spacing);
        container.setAutoresizingMask(sizable);
        container.setContentView(Some(&pieces));
        stage.addSubview(&container);
    } else {
        stage.addSubview(&pieces);
    }
    let content: Retained<TopAnchoredView> = unsafe {
        msg_send![TopAnchoredView::alloc(mtm), initWithFrame: NSRect::new(NSPoint::new(margin, margin), NSSize::new(pw, ph))]
    };
    // Placed by hand (apply_shape, layout_morph), never by autoresizing: a sizable view keeps its
    // margins as constraints, and the large bottom margin it has after a top-anchored morph would
    // stop the window shrinking back to the bar.
    content.setAutoresizingMask(NSAutoresizingMaskOptions::ViewNotSizable);
    // The layout is taller than the idle bar and than the panel mid-morph; show only what the
    // glass covers.
    content.setClipsToBounds(true);
    root.setFrame(rect);
    root.setAutoresizingMask(NSAutoresizingMaskOptions::ViewNotSizable);
    content.addSubview(&root);
    stage.addSubview(&content);
    panel.setContentView(Some(&stage));
    PIECES_HOST.with(|b| *b.borrow_mut() = Some(pieces));
    CONTENT.with(|c| *c.borrow_mut() = Some(Retained::into_super(content)));
    Some(Retained::into_super(panel))
}

/// One rounded piece of the panel's glass, spanning its full height. Only the backdrop: everything
/// drawn on it (icons, selection) is the Tish view.
#[derive(Clone)]
pub struct Piece {
    pub x: f64,
    pub width: f64,
    pub radius: f64,
}

fn current_shape() -> (f64, Vec<Piece>) {
    let (pw, ph) = PANEL_SIZE.with(|c| c.get());
    let (h, segs) = SHAPE.with(|s| s.borrow().clone());
    let h = if h > 0.0 { h.min(ph) } else { ph };
    let segs = if segs.is_empty() { vec![Piece { x: 0.0, width: pw, radius: theme::get().radius }] } else { segs };
    (h, segs)
}

/// Each piece's frame in window coordinates and its corner radius.
fn piece_frames() -> Vec<(NSRect, f64)> {
    let (h, segs) = current_shape();
    let m = margin();
    segs.iter()
        .map(|p| (NSRect::new(NSPoint::new(m + p.x, m), NSSize::new(p.width, h)), p.radius))
        .collect()
}

fn make_piece(mtm: MainThreadMarker) -> Retained<NSView> {
    let theme = theme::get();
    if GLASS.with(|g| g.get()) {
        let g = NSGlassEffectView::initWithFrame(NSGlassEffectView::alloc(mtm), NSRect::ZERO);
        g.setStyle(theme.glass_style);
        return Retained::into_super(g);
    }
    let fx = NSVisualEffectView::initWithFrame(NSVisualEffectView::alloc(mtm), NSRect::ZERO);
    fx.setMaterial(theme.material);
    fx.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
    fx.setState(NSVisualEffectState::Active);
    fx.setWantsLayer(true);
    if let Some(layer) = fx.layer() {
        layer.setMasksToBounds(true);
        layer.setBorderWidth(theme.edge_width);
    }
    // A tint layer over the vibrancy (its colour comes from the theme).
    let tint = NSView::initWithFrame(NSView::alloc(mtm), NSRect::ZERO);
    tint.setWantsLayer(true);
    fx.addSubview(&tint);
    Retained::into_super(fx)
}

/// The radius never exceeds half the height, so a squashed piece stays a capsule.
fn place_piece(piece: &NSView, rect: NSRect, radius: f64) {
    piece.setFrame(rect);
    let r = radius.min(rect.size.height / 2.0).min(rect.size.width / 2.0).max(0.0);
    if GLASS.with(|g| g.get()) {
        if let Some(g) = piece.downcast_ref::<NSGlassEffectView>() {
            g.setCornerRadius(r);
        }
        return;
    }
    if let Some(layer) = piece.layer() {
        layer.setCornerRadius(r);
    }
    let subs = piece.subviews();
    if subs.count() > 0 {
        subs.objectAtIndex(0).setFrame(piece.bounds());
    }
}

/// Size the panel to the current shape (keeping its top edge) and lay out its pieces.
fn apply_shape(w: &NSWindow, mtm: MainThreadMarker) {
    let (pw, _) = PANEL_SIZE.with(|c| c.get());
    let (h, _) = current_shape();
    let m = margin();
    let (ow, oh) = (pw + 2.0 * m, h + 2.0 * m);
    let f = w.frame();
    if (f.size.height - oh).abs() > 0.5 || (f.size.width - ow).abs() > 0.5 {
        let top = f.origin.y + f.size.height;
        w.setFrame_display(NSRect::new(NSPoint::new(f.origin.x, top - oh), NSSize::new(ow, oh)), true);
    }
    if let Some(c) = CONTENT.with(|c| c.borrow().clone()) {
        c.setFrame(NSRect::new(NSPoint::new(m, m), NSSize::new(pw, h)));
    }
    let frames = piece_frames();
    set_piece_count(frames.len(), mtm);
    PIECES.with(|p| {
        for (piece, &(rect, r)) in p.borrow().iter().zip(&frames) {
            place_piece(piece, rect, r);
        }
    });
    refresh_edge(w);
}

/// Add or remove pieces at the end; the first (the field, or the whole panel) always stays, so a
/// change of shape never recreates the glass under the field.
fn set_piece_count(n: usize, mtm: MainThreadMarker) {
    let Some(host) = PIECES_HOST.with(|b| b.borrow().clone()) else { return };
    PIECES.with(|p| {
        let mut pieces = p.borrow_mut();
        while pieces.len() > n {
            if let Some(old) = pieces.pop() {
                old.removeFromSuperview();
            }
        }
        while pieces.len() < n {
            let piece = make_piece(mtm);
            host.addSubview(&piece);
            pieces.push(piece);
        }
    });
}

pub fn set_panel_shape(h: f64, segs: Vec<Piece>) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let Some(w) = PANEL.with(|p| p.borrow().clone()) else {
        SHAPE.with(|s| *s.borrow_mut() = (h, segs));
        return;
    };
    let animate = SHOWN.with(|s| s.get())
        && w.isVisible()
        && theme::get().morph.secs > 0.0
        && std::env::var_os("MOO_NO_ANIMATION").is_none();
    if animate {
        stop_open_animation();
        stop_morph();
    }
    let (_, old_segs) = current_shape();
    let from: Vec<(NSRect, f64)> = PIECES.with(|p| p.borrow().iter().map(|v| v.frame()).collect::<Vec<_>>())
        .into_iter()
        .zip(old_segs.iter().map(|s| s.radius))
        .collect();
    let from_win = w.frame().size.height;
    SHAPE.with(|s| *s.borrow_mut() = (h, segs));
    if !animate || from.is_empty() {
        apply_shape(&w, mtm);
        return;
    }
    animate_morph(&w, from, from_win, mtm);
}

/// Apply the theme's tints for the current light / dark mode; layer and tint colours do not
/// update by themselves. Glass takes the bar or panel tint by the number of pieces; before macOS 26
/// pieces are vibrancy with a tint layer and an edge.
fn refresh_edge(w: &NSWindow) {
    let appearance = w.effectiveAppearance();
    let dark = appearance
        .bestMatchFromAppearancesWithNames(&NSArray::from_slice(&[unsafe { NSAppearanceNameAqua }, unsafe { NSAppearanceNameDarkAqua }]))
        .is_some_and(|n| n.to_string().contains("Dark"));
    let theme = theme::get();
    if GLASS.with(|g| g.get()) {
        let single = current_shape().1.len() == 1;
        let tint = if single { &theme.glass_tint_panel } else { &theme.glass_tint_bar }.resolve(dark);
        PIECES.with(|p| {
            for piece in p.borrow().iter() {
                if let Some(g) = piece.downcast_ref::<NSGlassEffectView>() {
                    g.setTintColor(tint.as_deref());
                }
            }
        });
        return;
    }
    let edge = theme.edge.resolve(dark);
    let tint = theme.vibrancy_tint.resolve(dark);
    PIECES.with(|p| {
        for piece in p.borrow().iter() {
            if let Some(layer) = piece.layer() {
                layer.setBorderColor(edge.as_ref().map(|c| c.CGColor()).as_deref());
            }
            let subs = piece.subviews();
            if let Some(layer) = (subs.count() > 0).then(|| subs.objectAtIndex(0)).and_then(|t| t.layer()) {
                layer.setBackgroundColor(tint.as_ref().map(|c| c.CGColor()).as_deref());
            }
        }
    });
    w.invalidateShadow();
}

// ── Opening animation ───────────────────────────────────────────────────────

/// Damped spring from 0 to 1 `t` seconds in; below critical damping it overshoots past 1 before
/// settling, at or above it it eases in without overshoot.
fn spring(t: f64, s: theme::Spring) -> f64 {
    if t <= 0.0 {
        return 0.0;
    }
    let w0 = 2.0 * std::f64::consts::PI / s.response.max(1e-3);
    let z = s.damping.max(0.0);
    if z >= 1.0 {
        return 1.0 - (-w0 * t).exp() * (1.0 + w0 * t);
    }
    let wd = w0 * (1.0 - z * z).sqrt();
    1.0 - (-z * w0 * t).exp() * ((wd * t).cos() + z * w0 / wd * (wd * t).sin())
}

fn lerp_rect(a: NSRect, b: NSRect, p: f64) -> NSRect {
    let l = |x: f64, y: f64| x + (y - x) * p;
    NSRect::new(
        NSPoint::new(l(a.origin.x, b.origin.x), l(a.origin.y, b.origin.y)),
        NSSize::new(l(a.size.width, b.size.width).max(1.0), l(a.size.height, b.size.height).max(1.0)),
    )
}

/// The panel `t` seconds into opening: the first piece grows from its centre in both directions
/// and the others spring out of its right end, farthest first, so on glass they bead off it. The
/// Tish view (field, icons) fades in as the pieces arrive.
fn layout_open(t: f64) {
    let frames = piece_frames();
    let pieces: Vec<Retained<NSView>> = PIECES.with(|p| p.borrow().clone());
    if pieces.len() != frames.len() || frames.is_empty() {
        return;
    }
    let th = theme::get();
    let (sx, sy) = if frames.len() > 1 { th.open_scale_bar } else { th.open_scale_panel };
    let (f0, r0) = frames[0];
    let start0 = NSRect::new(
        NSPoint::new(f0.origin.x + f0.size.width * (1.0 - sx) / 2.0, f0.origin.y + f0.size.height * (1.0 - sy) / 2.0),
        NSSize::new(f0.size.width * sx, f0.size.height * sy),
    );
    let cur0 = lerp_rect(start0, f0, spring(t, th.open));
    place_piece(&pieces[0], cur0, r0);
    let n = pieces.len();
    for i in 1..n {
        let (target, r) = frames[i];
        let p = spring(t - (n - 1 - i) as f64 * th.stagger, th.open);
        let tucked = NSRect::new(
            NSPoint::new(cur0.origin.x + cur0.size.width - target.size.width - th.tuck, cur0.origin.y),
            NSSize::new(target.size.width, cur0.size.height),
        );
        place_piece(&pieces[i], lerp_rect(tucked, target, p), r);
        pieces[i].setAlphaValue((p * th.piece_fade).clamp(0.0, 1.0));
    }
    if let Some(c) = CONTENT.with(|c| c.borrow().clone()) {
        let shown = if th.fade_secs > 0.0 { (t - th.fade_delay) / th.fade_secs } else if t >= th.fade_delay { 1.0 } else { 0.0 };
        c.setAlphaValue(shown.clamp(0.0, 1.0));
    }
}

fn finish_open() {
    let Some(mtm) = MainThreadMarker::new() else { return };
    PIECES.with(|p| {
        for v in p.borrow().iter() {
            v.setAlphaValue(1.0);
        }
    });
    if let Some(c) = CONTENT.with(|c| c.borrow().clone()) {
        c.setAlphaValue(1.0);
    }
    if let Some(w) = PANEL.with(|p| p.borrow().clone()) {
        apply_shape(&w, mtm);
    }
}

fn stop_open_animation() {
    if let Some(timer) = OPEN_ANIM.with(|a| a.borrow_mut().take()) {
        timer.invalidate();
        finish_open();
    }
}

/// Spring the panel open, stepped on a 120 Hz timer so glass re-shapes (and melts) every frame.
fn animate_open() {
    stop_open_animation();
    let secs = theme::get().open.secs;
    if secs <= 0.0 || std::env::var_os("MOO_NO_ANIMATION").is_some() {
        return;
    }
    layout_open(0.0);
    let t0 = std::time::Instant::now();
    let tick = RcBlock::new(move |timer: NonNull<NSTimer>| {
        let t = t0.elapsed().as_secs_f64();
        if t < secs {
            layout_open(t);
            return;
        }
        unsafe { timer.as_ref() }.invalidate();
        OPEN_ANIM.with(|a| a.borrow_mut().take());
        finish_open();
    });
    let timer = unsafe { NSTimer::scheduledTimerWithTimeInterval_repeats_block(1.0 / 120.0, true, &tick) };
    OPEN_ANIM.with(|a| *a.borrow_mut() = Some(timer));
}

/// One frame of a morph `p` of the way (springs past 1). Pieces in both shapes move; the field
/// (piece 0) grows into the panel or shrinks back. A piece only in the old shape stays put and
/// fades, melting into the panel as it grows over it; one only in the new shape appears in place,
/// beading off as the panel shrinks away from it. The layout is clipped to the first piece's height.
fn layout_morph(p: f64, from: &[(NSRect, f64)], to: &[(NSRect, f64)], heights: (f64, f64), win: f64) {
    let th = theme::get();
    PIECES.with(|ps| {
        for (i, v) in ps.borrow().iter().enumerate() {
            match (from.get(i), to.get(i)) {
                (Some(&(a, ra)), Some(&(b, rb))) => {
                    place_piece(v, lerp_rect(a, b, p), ra + (rb - ra) * p.clamp(0.0, 1.0));
                    v.setAlphaValue(1.0);
                }
                (Some(&(a, ra)), None) => {
                    place_piece(v, a, ra);
                    v.setAlphaValue((1.0 - p * th.morph_fade).clamp(0.0, 1.0));
                }
                (None, Some(&(b, rb))) => {
                    place_piece(v, b, rb);
                    v.setAlphaValue((p * th.morph_fade).clamp(0.0, 1.0));
                }
                (None, None) => {}
            }
        }
    });
    if let Some(c) = CONTENT.with(|c| c.borrow().clone()) {
        let (pw, _) = PANEL_SIZE.with(|c| c.get());
        let h = (heights.0 + (heights.1 - heights.0) * p).max(1.0);
        c.setFrame(NSRect::new(NSPoint::new(th.margin, win - th.margin - h), NSSize::new(pw, h)));
    }
}

/// Spring from the shape the pieces have now (`from`, in a window `from_win` tall) to the current
/// shape. The window takes the taller of the two for the duration, keeping its top edge.
fn animate_morph(w: &NSWindow, from: Vec<(NSRect, f64)>, from_win: f64, mtm: MainThreadMarker) {
    let (pw, _) = PANEL_SIZE.with(|c| c.get());
    let (h_to, _) = current_shape();
    let m = margin();
    let morph = theme::get().morph;
    let to_win = h_to + 2.0 * m;
    let win = from_win.max(to_win);
    let f = w.frame();
    if (f.size.height - win).abs() > 0.5 {
        let top = f.origin.y + f.size.height;
        w.setFrame_display(NSRect::new(NSPoint::new(f.origin.x, top - win), NSSize::new(pw + 2.0 * m, win)), false);
    }
    let shift = |r: NSRect, dy: f64| NSRect::new(NSPoint::new(r.origin.x, r.origin.y + dy), r.size);
    let from: Vec<(NSRect, f64)> = from.into_iter().map(|(r, rad)| (shift(r, win - from_win), rad)).collect();
    let to: Vec<(NSRect, f64)> = piece_frames().into_iter().map(|(r, rad)| (shift(r, win - to_win), rad)).collect();
    let heights = (from_win - 2.0 * m, h_to);
    set_piece_count(from.len().max(to.len()), mtm);
    refresh_edge(w);
    layout_morph(0.0, &from, &to, heights, win);
    let t0 = std::time::Instant::now();
    let tick = RcBlock::new(move |timer: NonNull<NSTimer>| {
        let t = t0.elapsed().as_secs_f64();
        if t < morph.secs {
            layout_morph(spring(t, morph), &from, &to, heights, win);
            return;
        }
        unsafe { timer.as_ref() }.invalidate();
        MORPH_ANIM.with(|a| a.borrow_mut().take());
        finish_morph();
    });
    let timer = unsafe { NSTimer::scheduledTimerWithTimeInterval_repeats_block(1.0 / 120.0, true, &tick) };
    MORPH_ANIM.with(|a| *a.borrow_mut() = Some(timer));
}

fn finish_morph() {
    let Some(mtm) = MainThreadMarker::new() else { return };
    if let Some(w) = PANEL.with(|p| p.borrow().clone()) {
        apply_shape(&w, mtm);
    }
    PIECES.with(|p| {
        for v in p.borrow().iter() {
            v.setAlphaValue(1.0);
        }
    });
}

fn stop_morph() {
    if let Some(timer) = MORPH_ANIM.with(|a| a.borrow_mut().take()) {
        timer.invalidate();
        finish_morph();
    }
}

fn debug_log(msg: &str) {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    if std::env::var_os("MOO_DEBUG").is_some() {
        let t = START.get_or_init(std::time::Instant::now).elapsed().as_secs_f64() * 1000.0;
        eprintln!("moo: [{t:.1} ms] {msg}");
    }
}

fn hide_on_resign_key(w: &NSWindow) {
    let center = NSNotificationCenter::defaultCenter();
    let resign = RcBlock::new(|_n: NonNull<NSNotification>| {
        debug_log("panel key=false");
        if SHOWN.with(|s| s.get()) && PREVIEW_PATH.with(|p| !p.borrow().is_empty()) && quick_look_visible() {
            DispatchQueue::main().exec_async(|| {
                if let Some(w) = MainThreadMarker::new().and_then(main_window) {
                    w.makeKeyWindow();
                }
            });
            return;
        }
        if SHOWN.with(|s| s.get()) {
            hide();
        }
    });
    let became = RcBlock::new(|_n: NonNull<NSNotification>| debug_log("panel key=true"));
    unsafe {
        std::mem::forget(center.addObserverForName_object_queue_usingBlock(
            Some(NSWindowDidResignKeyNotification),
            Some(w),
            None,
            &resign,
        ));
        std::mem::forget(center.addObserverForName_object_queue_usingBlock(
            Some(NSWindowDidBecomeKeyNotification),
            Some(w),
            None,
            &became,
        ));
    }
}

fn style_panel(w: &NSWindow, mtm: MainThreadMarker) {
    w.setTitlebarAppearsTransparent(true);
    w.setTitleVisibility(NSWindowTitleVisibility::Hidden);
    for b in [
        NSWindowButton::CloseButton,
        NSWindowButton::MiniaturizeButton,
        NSWindowButton::ZoomButton,
    ] {
        if let Some(btn) = w.standardWindowButton(b) {
            btn.setHidden(true);
        }
    }
    w.setMovableByWindowBackground(true);
    // NSFloatingWindowLevel
    w.setLevel(3);
    w.setCollectionBehavior(
        NSWindowCollectionBehavior::MoveToActiveSpace
            | NSWindowCollectionBehavior::FullScreenAuxiliary,
    );
    let (pw, ph) = PANEL_SIZE.with(|c| c.get());
    let m = margin();
    w.setContentSize(NSSize::new(pw + 2.0 * m, ph + 2.0 * m));
    position_panel(w, mtm);
}

fn first_editable_text_field(v: &NSView) -> Option<Retained<NSTextField>> {
    let subs = v.subviews();
    for i in 0..subs.count() {
        let child = subs.objectAtIndex(i);
        if let Ok(tf) = child.clone().downcast::<NSTextField>() {
            if tf.isEditable() {
                return Some(tf);
            }
        }
        if let Some(found) = first_editable_text_field(&child) {
            return Some(found);
        }
    }
    None
}

/// ⌫ in an empty field leaves the category, as in Spotlight.
fn search_field_empty() -> bool {
    MainThreadMarker::new()
        .and_then(main_window)
        .and_then(|w| w.contentView())
        .and_then(|c| first_editable_text_field(&c))
        .is_some_and(|tf| tf.stringValue().length() == 0)
}

fn focus_search(w: &NSWindow) {
    let Some(content) = w.contentView() else { return };
    if let Some(tf) = first_editable_text_field(&content) {
        w.makeFirstResponder(Some(&tf));
        unsafe { tf.selectText(None) };
    }
}

/// A re-render that rebuilds the field leaves the panel itself as first responder;
/// hand focus back to the search field with the caret after what was typed.
fn restore_focus() {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let Some(w) = main_window(mtm) else { return };
    if !SHOWN.with(|s| s.get()) || !w.isVisible() {
        return;
    }
    if w.firstResponder().is_some_and(|r| !r.isKindOfClass(NSWindow::class())) {
        return;
    }
    let Some(tf) = w.contentView().and_then(|c| first_editable_text_field(&c)) else { return };
    w.makeFirstResponder(Some(&tf));
    let end = tf.stringValue().length();
    let editor: Option<Retained<AnyObject>> = unsafe { msg_send![&*tf, currentEditor] };
    if let Some(ed) = editor {
        let _: () = unsafe { msg_send![&*ed, setSelectedRange: NSRange::new(end, 0)] };
    }
}

pub fn show() {
    let Some(mtm) = MainThreadMarker::new() else { return };
    // A CLI request can arrive before the panel exists (the socket opens first); setup shows it.
    if PANEL.with(|p| p.borrow().is_none()) {
        SHOWN.with(|s| s.set(true));
        return;
    }
    if let Some(w) = main_window(mtm) {
        position_panel(&w, mtm);
        refresh_edge(&w);
        if !w.isVisible() && PANEL.with(|p| p.borrow().is_some()) {
            animate_open();
        }
        w.orderFrontRegardless();
        w.makeKeyWindow();
        focus_search(&w);
    }
    SHOWN.with(|s| s.set(true));
    debug_log("panel ordered front");
    defer_callback("show", "show");
}

/// Test hook for `moo type`: feed `text` to whatever has focus through `insertText:`, one
/// character every 120 ms, reporting where focus is after each.
pub fn type_text(text: String) {
    let chars: Vec<char> = text.chars().collect();
    type_step(chars, 0);
}

fn type_step(chars: Vec<char>, i: usize) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let Some(w) = main_window(mtm) else { return };
    let responder = |w: &NSWindow| w.firstResponder().map(|r| r.class().name().to_string_lossy().into_owned()).unwrap_or_default();
    let field = || w.contentView().and_then(|c| first_editable_text_field(&c)).map(|tf| tf.stringValue().to_string()).unwrap_or_default();
    if i >= chars.len() {
        eprintln!("moo: TYPE done: responder={} field={:?}", responder(&w), field());
        return;
    }
    let before = responder(&w);
    if let Some(r) = w.firstResponder() {
        let s = NSString::from_str(&chars[i].to_string());
        let _: () = unsafe { msg_send![&*r, insertText: &*s] };
    }
    eprintln!("moo: TYPE {:?}: before={before} after={} field={:?}", chars[i], responder(&w), field());
    let when = DispatchTime::try_from(std::time::Duration::from_millis(120)).unwrap_or(DispatchTime::NOW);
    let _ = DispatchQueue::main().after(when, move || type_step(chars, i + 1));
}

pub fn hide() {
    let Some(mtm) = MainThreadMarker::new() else { return };
    SHOWN.with(|s| s.set(false));
    close_quick_look();
    stop_open_animation();
    stop_morph();
    if let Some(w) = main_window(mtm) {
        w.orderOut(None);
    }
    // A hidden window gets no mouseExited, so the hovered row would still be lit when it shows.
    if let Some(cls) = AnyClass::get(c"TishHoverButton") {
        let _: () = unsafe { msg_send![cls, clearHover] };
    }
}

fn panel_has_keys() -> bool {
    MainThreadMarker::new()
        .and_then(main_window)
        .is_some_and(|w| w.isVisible() && w.isKeyWindow())
}

pub fn toggle() {
    if SHOWN.with(|s| s.get()) && panel_has_keys() {
        hide();
    } else {
        show();
    }
}

pub fn quit() {
    let Some(mtm) = MainThreadMarker::new() else { return };
    crate::cli::stop_serving();
    app(mtm).terminate(None);
}

// ── Key routing ─────────────────────────────────────────────────────────────

/// An accessory app has no Edit menu, so Cmd/Ctrl edit keys never reach the field editor unless
/// they are sent down the responder chain by hand.
fn edit_shortcut(e: &NSEvent) -> bool {
    let flags = e.modifierFlags();
    let cmd = flags.contains(NSEventModifierFlags::Command);
    let ctrl = flags.contains(NSEventModifierFlags::Control);
    let shift = flags.contains(NSEventModifierFlags::Shift);
    if !cmd && !ctrl {
        return false;
    }
    let Some(chars) = e.charactersIgnoringModifiers() else { return false };
    let action = match (chars.to_string().to_lowercase().as_str(), shift) {
        ("c", false) => sel!(copy:),
        ("v", false) => sel!(paste:),
        ("x", false) => sel!(cut:),
        ("z", false) => sel!(undo:),
        ("z", true) | ("y", false) => sel!(redo:),
        ("a", false) if cmd => sel!(selectAll:),
        _ => return false,
    };
    let Some(mtm) = MainThreadMarker::new() else { return false };
    unsafe { app(mtm).sendAction_to_from(action, None, None) }
}

thread_local! {
    static ARROWS: Cell<bool> = const { Cell::new(false) };
}

/// While on, plain ← and → are launcher keys rather than cursor movement in the search field.
pub fn set_arrow_keys(on: bool) {
    ARROWS.with(|a| a.set(on));
}

fn install_key_monitor() {
    if MONITOR.with(|m| m.borrow().is_some()) {
        return;
    }
    let block = RcBlock::new(|ev: NonNull<NSEvent>| -> *mut NSEvent {
        let e = unsafe { ev.as_ref() };
        if RECORDING.with(|r| r.get()) {
            if let Some(spec) = recorded_key(e) {
                defer_callback("key", format!("record:{spec}"));
            }
            return std::ptr::null_mut();
        }
        if edit_shortcut(e) {
            return std::ptr::null_mut();
        }
        let flags = e.modifierFlags();
        let ctrl = flags.contains(NSEventModifierFlags::Control);
        let cmd = flags.contains(NSEventModifierFlags::Command);
        let alt = flags.contains(NSEventModifierFlags::Option);
        if cmd && !ctrl && !alt {
            let chars = e.charactersIgnoringModifiers().map(|c| c.to_string().to_lowercase()).unwrap_or_default();
            let named = match chars.as_str() {
                "1" => Some("cmd+1"),
                "2" => Some("cmd+2"),
                "3" => Some("cmd+3"),
                "4" => Some("cmd+4"),
                "r" => Some("cmd+r"),
                "l" => Some("cmd+l"),
                "h" => Some("cmd+h"),
                "k" => Some("cmd+k"),
                "o" => Some("cmd+o"),
                "y" => Some("cmd+y"),
                _ => None,
            };
            if let Some(n) = named {
                defer_callback("key", n);
                return std::ptr::null_mut();
            }
        }
        let arrows = ARROWS.with(|a| a.get()) && !cmd && !alt && !flags.contains(NSEventModifierFlags::Shift);
        let name = match (e.keyCode(), ctrl) {
            (123, false) if arrows => Some("left"),
            (124, false) if arrows => Some("right"),
            (125, _) | (45, true) => Some("down"),
            (126, _) | (35, true) => Some("up"),
            (36, _) | (76, _) if cmd => Some("cmd+enter"),
            (36, _) | (76, _) if alt => Some("alt+enter"),
            (36, _) | (76, _) => Some("enter"),
            (51, _) if cmd => Some("cmd+delete"),
            (51, false) if !alt && search_field_empty() => Some("delete-empty"),
            (53, _) => Some("escape"),
            (48, _) if flags.contains(NSEventModifierFlags::Shift) => Some("shift+tab"),
            (48, _) => Some("tab"),
            _ => None,
        };
        match name {
            Some(n) => {
                defer_callback("key", n);
                std::ptr::null_mut()
            }
            None => ev.as_ptr(),
        }
    });
    let monitor =
        unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::KeyDown, &block) };
    MONITOR.with(|m| *m.borrow_mut() = monitor);
    install_scroll_monitor();
}

/// Trackpad points that make one row; a mouse wheel moves a row per line.
const SCROLL_POINTS: f64 = 18.0;

/// Trackpad and mouse wheel scrolling arrive as `onKey("scroll:<rows>")`, positive towards the end
/// of the list. The lists are drawn by Tish, so there is no scroll view to hand them to.
fn install_scroll_monitor() {
    if SCROLL_MONITOR.with(|m| m.borrow().is_some()) {
        return;
    }
    let block = RcBlock::new(|ev: NonNull<NSEvent>| -> *mut NSEvent {
        let e = unsafe { ev.as_ref() };
        let per_row = if e.hasPreciseScrollingDeltas() { SCROLL_POINTS } else { 1.0 };
        // scrollingDeltaY follows the natural-scrolling setting: positive moves the content down,
        // which shows earlier rows.
        let total = SCROLL_REST.with(|r| r.get()) - e.scrollingDeltaY() / per_row;
        let rows = total.trunc();
        SCROLL_REST.with(|r| r.set(total - rows));
        if rows != 0.0 {
            defer_callback("key", format!("scroll:{}", rows as i64));
        }
        ev.as_ptr()
    });
    let monitor =
        unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::ScrollWheel, &block) };
    SCROLL_MONITOR.with(|m| *m.borrow_mut() = monitor);
}

// ── Snippet keywords typed in other apps ────────────────────────────────────

thread_local! {
    static TYPED: RefCell<crate::snippets::Typed> = RefCell::new(Default::default());
    static TYPED_MONITOR: RefCell<Option<Retained<AnyObject>>> = const { RefCell::new(None) };
    static ON_SNIPPET: RefCell<Option<Value>> = const { RefCell::new(None) };
}

/// Watch what is typed in other apps for `keywords`; `cb(keyword)` runs once the app has inserted
/// the keyword's last character. No keywords stops watching. Other apps' key events only arrive
/// with Accessibility, and none arrive while a password field has focus.
pub fn watch_snippets(keywords: Vec<String>, cb: Value) {
    TYPED.with(|t| t.borrow_mut().set_keywords(keywords));
    ON_SNIPPET.with(|c| *c.borrow_mut() = Some(cb));
    let watching = !TYPED.with(|t| t.borrow().is_empty());
    let monitor = TYPED_MONITOR.with(|m| m.borrow_mut().take());
    if !watching {
        if let Some(m) = monitor {
            unsafe { NSEvent::removeMonitor(&m) };
        }
        return;
    }
    if monitor.is_some() {
        TYPED_MONITOR.with(|m| *m.borrow_mut() = monitor);
        return;
    }
    let block = RcBlock::new(|ev: NonNull<NSEvent>| {
        if let Some(keyword) = typed_keyword(unsafe { ev.as_ref() }) {
            let when = DispatchTime::try_from(std::time::Duration::from_millis(40)).unwrap_or(DispatchTime::NOW);
            let _ = DispatchQueue::main().after(when, move || {
                let Some(Value::Function(f)) = ON_SNIPPET.with(|c| c.borrow().clone()) else { return };
                with_ui(|| {
                    let _ = f.call(&[Value::String(keyword.as_str().into())]);
                });
            });
        }
    });
    let mask = NSEventMask::KeyDown | NSEventMask::LeftMouseDown | NSEventMask::RightMouseDown | NSEventMask::OtherMouseDown;
    let monitor = NSEvent::addGlobalMonitorForEventsMatchingMask_handler(mask, &block);
    TYPED_MONITOR.with(|m| *m.borrow_mut() = monitor);
}

/// Feed one event to the typed-text buffer; a click, a shortcut or a key that moves the cursor
/// starts it over.
fn typed_keyword(e: &NSEvent) -> Option<String> {
    TYPED.with(|t| {
        let mut t = t.borrow_mut();
        let flags = e.modifierFlags();
        if e.r#type() != NSEventType::KeyDown || flags.intersects(NSEventModifierFlags::Command | NSEventModifierFlags::Control) {
            t.reset();
            return None;
        }
        match e.keyCode() {
            51 => {
                t.backspace();
                None
            }
            36 | 48 | 53 | 76 | 115 | 116 | 117 | 119 | 121 | 123..=126 => {
                t.reset();
                None
            }
            _ => t.push(&e.characters().map(|c| c.to_string()).unwrap_or_default()),
        }
    })
}

// ── Setup once the run loop is live ─────────────────────────────────────────

/// Called before `macos.run`: the window does not exist yet, so finish on the main queue.
pub fn schedule_setup() {
    DispatchQueue::main().exec_async(|| finish_setup(0));
}

fn finish_setup(attempt: u32) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let Some(w) = host_window(mtm).and_then(|hw| adopt_into_panel(&hw, mtm)) else {
        if attempt < 100 {
            std::thread::sleep(std::time::Duration::from_millis(10));
            DispatchQueue::main().exec_async(move || finish_setup(attempt + 1));
        }
        return;
    };
    PANEL.with(|p| *p.borrow_mut() = Some(w.clone()));
    app(mtm).setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    style_panel(&w, mtm);
    apply_shape(&w, mtm);
    hide_on_resign_key(&w);
    install_key_monitor();
    if !START_HIDDEN.with(|c| c.get()) || SHOWN.with(|s| s.get()) {
        show();
    }
}

// ── Global hotkey (Carbon; needs no Accessibility permission) ───────────────

#[repr(C)]
struct EventTypeSpec {
    event_class: u32,
    event_kind: u32,
}

#[repr(C)]
struct EventHotKeyID {
    signature: u32,
    id: u32,
}

type EventHandlerProc = extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> i32;

#[link(name = "Carbon", kind = "framework")]
extern "C" {
    fn GetApplicationEventTarget() -> *mut c_void;
    fn InstallEventHandler(
        target: *mut c_void,
        handler: EventHandlerProc,
        num_types: u32,
        list: *const EventTypeSpec,
        user_data: *mut c_void,
        out_ref: *mut *mut c_void,
    ) -> i32;
    fn RegisterEventHotKey(
        key_code: u32,
        modifiers: u32,
        id: EventHotKeyID,
        target: *mut c_void,
        options: u32,
        out_ref: *mut *mut c_void,
    ) -> i32;
    fn UnregisterEventHotKey(hotkey: *mut c_void) -> i32;
    fn GetEventParameter(
        event: *mut c_void,
        name: u32,
        desired_type: u32,
        actual_type: *mut u32,
        size: usize,
        actual_size: *mut usize,
        data: *mut c_void,
    ) -> i32;
}

const fn fourcc(s: &[u8; 4]) -> u32 {
    ((s[0] as u32) << 24) | ((s[1] as u32) << 16) | ((s[2] as u32) << 8) | (s[3] as u32)
}

/// The launcher's own action: toggle the panel directly, without a round trip through Tish.
pub const TOGGLE: &str = "toggle";

/// One hotkey as the user wrote it; registered once per logical modifier combination.
struct Binding {
    action: String,
    /// What the hotkey runs, for messages: "Google", "the Moo launcher".
    label: String,
    display: String,
    /// `(Carbon hotkey ref, key code, logical modifiers)` per registration.
    refs: Vec<(*mut c_void, u32, u32)>,
}

thread_local! {
    /// Binding id -> binding. Carbon hotkey ids map to binding ids through `HOTKEY_OWNER`.
    static BINDINGS: RefCell<HashMap<u32, Binding>> = RefCell::new(HashMap::new());
    static HOTKEY_OWNER: RefCell<HashMap<u32, u32>> = RefCell::new(HashMap::new());
    static NEXT_BINDING: Cell<u32> = const { Cell::new(1) };
    static RECORDING: Cell<bool> = const { Cell::new(false) };
}

extern "C" fn on_hotkey(_next: *mut c_void, event: *mut c_void, _user: *mut c_void) -> i32 {
    let mut hk = EventHotKeyID { signature: 0, id: 0 };
    let st = unsafe {
        GetEventParameter(
            event,
            fourcc(b"----"),
            fourcc(b"hkid"),
            std::ptr::null_mut(),
            std::mem::size_of::<EventHotKeyID>(),
            std::ptr::null_mut(),
            &mut hk as *mut EventHotKeyID as *mut c_void,
        )
    };
    let action = if st == 0 {
        HOTKEY_OWNER
            .with(|o| o.borrow().get(&hk.id).copied())
            .and_then(|b| BINDINGS.with(|m| m.borrow().get(&b).map(|b| b.action.clone())))
    } else {
        None
    };
    let action = action.unwrap_or_else(|| TOGGLE.to_string());
    debug_log(&format!("hotkey {action} shown={} key={}", SHOWN.with(|s| s.get()), panel_has_keys()));
    if action == TOGGLE {
        toggle();
    } else {
        defer_callback("hotkey", action);
    }
    0
}

pub struct Hotkey {
    /// Binding id, for `unregister_hotkey`.
    pub id: u32,
    /// What the user presses, as printed on the keys: `⌘Space`.
    pub display: String,
    /// What was registered after the keyboards' modifier mappings: `ctrl+space`.
    pub registered: Vec<String>,
}

/// The logical combinations `spec` needs, or why it cannot be bound: a system shortcut or another
/// Moo hotkey owns it, or a modifier is remapped away on every keyboard.
fn plan_hotkey(spec: &str) -> Result<(keys::Spec, String, Vec<u32>, Vec<String>), String> {
    let s = keys::parse(spec)?;
    let display = keys::display(s.mods, s.key);
    let combos = keymap::logical_combos(s.mods, &keymap::active_mappings());
    if combos.is_empty() {
        return Err(format!("{display}: a modifier is remapped to a non-modifier key on every keyboard"));
    }
    let owner = BINDINGS.with(|m| {
        m.borrow()
            .values()
            .find(|b| b.refs.iter().any(|(_, c, md)| *c == s.code && combos.contains(md)))
            .map(|b| b.label.clone())
    });
    if let Some(label) = owner {
        return Err(format!("{display} is already bound to {label}"));
    }
    let mut free = Vec::new();
    let mut taken = Vec::new();
    for mods in combos {
        match keymap::system_shortcut(s.code, mods) {
            Some(owner) => taken.push(format!("{} is macOS \"{owner}\"", keymap::spec_name(mods, s.key))),
            None => free.push(mods),
        }
    }
    if free.is_empty() {
        return Err(format!("{display} unavailable: {}", taken.join("; ")));
    }
    Ok((s, display, free, taken))
}

/// `Ok(display)` when `spec` could be bound now, else the reason it cannot.
pub fn check_hotkey(spec: &str) -> Result<String, String> {
    plan_hotkey(spec).map(|(_, display, _, _)| display)
}

/// Register `spec` (keys as printed) for whatever the connected keyboards' modifier mappings make
/// those keys produce. Combinations an enabled system shortcut owns are skipped: macOS would
/// accept them and then never deliver the keypress. `action` is `TOGGLE` for the launcher;
/// anything else is passed to the Tish `onHotkey` callback. `label` names the target in conflict
/// messages; empty uses `action`.
pub fn register_hotkey(spec: &str, action: &str, label: &str) -> Result<Hotkey, String> {
    let (s, display, combos, mut taken) = plan_hotkey(spec)?;
    let binding_id = NEXT_BINDING.with(|n| n.replace(n.get() + 1));
    let mut refs = Vec::new();
    let mut registered = Vec::new();
    unsafe {
        let target = GetApplicationEventTarget();
        if !HOTKEY_HANDLER.with(|c| c.get()) {
            let spec = EventTypeSpec { event_class: fourcc(b"keyb"), event_kind: 5 };
            let st = InstallEventHandler(target, on_hotkey, 1, &spec, std::ptr::null_mut(), std::ptr::null_mut());
            if st != 0 {
                return Err(format!("InstallEventHandler failed: {st}"));
            }
            HOTKEY_HANDLER.with(|c| c.set(true));
        }
        for mods in combos {
            let name = keymap::spec_name(mods, s.key);
            let mut out = std::ptr::null_mut();
            let hk_id = HOTKEY_IDS.with(|n| n.replace(n.get() + 1));
            let id = EventHotKeyID { signature: fourcc(b"nmbl"), id: hk_id };
            match RegisterEventHotKey(s.code, mods, id, target, 0, &mut out) {
                0 => {
                    HOTKEY_OWNER.with(|o| o.borrow_mut().insert(hk_id, binding_id));
                    refs.push((out, s.code, mods));
                    registered.push(name);
                }
                st => taken.push(format!("{name} is taken by another app (RegisterEventHotKey: {st})")),
            }
        }
    }
    if registered.is_empty() {
        return Err(format!("{display} unavailable: {}", taken.join("; ")));
    }
    debug_log(&format!("hotkey {display} -> {action} registered as {}", registered.join(", ")));
    let label = match label {
        "" if action == TOGGLE => "the Moo launcher".to_string(),
        "" => action.to_string(),
        l => l.to_string(),
    };
    BINDINGS.with(|m| m.borrow_mut().insert(binding_id, Binding { action: action.to_string(), label, display: display.clone(), refs }));
    Ok(Hotkey { id: binding_id, display, registered })
}

pub fn unregister_hotkey(id: u32) -> bool {
    let Some(b) = BINDINGS.with(|m| m.borrow_mut().remove(&id)) else { return false };
    for (r, _, _) in b.refs {
        unsafe { UnregisterEventHotKey(r) };
    }
    HOTKEY_OWNER.with(|o| o.borrow_mut().retain(|_, owner| *owner != id));
    debug_log(&format!("hotkey {} -> {} unregistered", b.display, b.action));
    true
}

/// While recording, every keypress in the panel goes to `onKey` as `record:<spec>` (or
/// `record:escape` / `record:return` / `record:delete` for those keys alone) instead of the field.
pub fn set_recording(on: bool) {
    RECORDING.with(|r| r.set(on));
}

fn carbon_mods(flags: NSEventModifierFlags) -> u32 {
    let mut m = 0;
    for (f, bit) in [
        (NSEventModifierFlags::Command, keymap::CMD),
        (NSEventModifierFlags::Shift, keymap::SHIFT),
        (NSEventModifierFlags::Option, keymap::OPT),
        (NSEventModifierFlags::Control, keymap::CTRL),
    ] {
        if flags.contains(f) {
            m |= bit;
        }
    }
    m
}

fn recorded_key(e: &NSEvent) -> Option<String> {
    let mods = carbon_mods(e.modifierFlags());
    let spec = keys::from_event(e.keyCode() as u32, mods)?;
    match spec.as_str() {
        "escape" | "return" | "delete" => Some(spec),
        _ => keys::parse(&spec).ok().map(|_| spec),
    }
}

// ── Launch and icons ────────────────────────────────────────────────────────

pub fn launch(target: &str) -> bool {
    let s = NSString::from_str(target);
    let url = if target.contains("://") {
        NSURL::URLWithString(&s)
    } else {
        Some(NSURL::fileURLWithPath(&s))
    };
    url.is_some_and(|u| NSWorkspace::sharedWorkspace().openURL(&u))
}

pub fn reveal(path: &str) -> bool {
    if !std::path::Path::new(path).exists() {
        return false;
    }
    let url = NSURL::fileURLWithPath(&NSString::from_str(path));
    NSWorkspace::sharedWorkspace().activateFileViewerSelectingURLs(&NSArray::from_retained_slice(&[url]));
    true
}

pub fn clipboard_text() -> String {
    NSPasteboard::generalPasteboard()
        .stringForType(unsafe { NSPasteboardTypeString })
        .map(|s| s.to_string())
        .unwrap_or_default()
}

pub fn copy_text(text: &str) -> bool {
    let pb = NSPasteboard::generalPasteboard();
    pb.clearContents();
    let ok = pb.setString_forType(&NSString::from_str(text), unsafe { NSPasteboardTypeString });
    clip::note_own_change();
    ok
}

// ── Status bar item ─────────────────────────────────────────────────────────

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "MooMenuTarget"]
    struct MenuTarget;

    impl MenuTarget {
        #[unsafe(method(statusClicked:))]
        fn status_clicked(&self, _sender: Option<&AnyObject>) {
            let Some(mtm) = MainThreadMarker::new() else { return };
            let menu_click = NSApplication::sharedApplication(mtm).currentEvent().is_some_and(|ev| {
                ev.r#type() == NSEventType::RightMouseUp || ev.modifierFlags().contains(NSEventModifierFlags::Control)
            });
            if !menu_click {
                show();
                return;
            }
            STATUS.with(|s| {
                let s = s.borrow();
                let Some(st) = s.as_ref() else { return };
                // A status item opens its menu below itself on click; set it just for this click.
                st.item.setMenu(Some(&st.menu));
                if let Some(button) = st.item.button(mtm) {
                    unsafe { button.performClick(None) };
                }
                st.item.setMenu(None);
            });
        }

        #[unsafe(method(openSettings:))]
        fn open_settings(&self, _sender: Option<&AnyObject>) {
            STATUS_MENU.with(|h| {
                if let Some(h) = h.borrow().as_ref() {
                    h("settings");
                }
            });
        }

        #[unsafe(method(quitMoo:))]
        fn quit_moo(&self, _sender: Option<&AnyObject>) {
            quit();
        }
    }
);

impl MenuTarget {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        unsafe { msg_send![Self::alloc(mtm), init] }
    }
}

pub struct StatusItem {
    item: Retained<NSStatusItem>,
    menu: Retained<NSMenu>,
    _target: Retained<MenuTarget>,
}

/// Menu bar icon, installed once the run loop is live. A click shows the panel; a right click
/// (or Control-click) opens Settings… / Quit Moo, and Settings calls `on_menu("settings")`.
/// `hotkey` goes in the tooltip as a reminder.
pub fn status_item(hotkey: &str, symbol: &str, on_menu: Box<dyn Fn(&str)>) -> bool {
    if STATUS.with(|s| s.borrow().is_some()) {
        let Some(mtm) = MainThreadMarker::new() else { return false };
        STATUS.with(|s| {
            if let Some(button) = s.borrow().as_ref().and_then(|st| st.item.button(mtm)) {
                set_status_tooltip(&button, hotkey);
            }
        });
        return true;
    }
    STATUS_MENU.with(|h| *h.borrow_mut() = Some(on_menu));
    let (hotkey, symbol) = (hotkey.to_string(), symbol.to_string());
    DispatchQueue::main().exec_async(move || install_status_item(&hotkey, &symbol));
    true
}

fn install_status_item(hotkey: &str, symbol: &str) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    if STATUS.with(|s| s.borrow().is_some()) {
        return;
    }
    // NSVariableStatusItemLength
    let item = NSStatusBar::systemStatusBar().statusItemWithLength(-1.0);
    if let Some(button) = item.button(mtm) {
        let desc = NSString::from_str("Moo");
        match NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str(symbol), Some(&desc)) {
            Some(img) => {
                img.setTemplate(true);
                button.setImage(Some(&img));
            }
            None => button.setTitle(&desc),
        }
    }
    let target = MenuTarget::new(mtm);
    if let Some(button) = item.button(mtm) {
        set_status_tooltip(&button, hotkey);
        unsafe {
            button.setTarget(Some(&target));
            button.setAction(Some(sel!(statusClicked:)));
        }
        button.sendActionOn(NSEventMask::LeftMouseUp | NSEventMask::RightMouseUp);
    }
    let menu = NSMenu::new(mtm);
    let entries = [(Some("Settings…"), Some(sel!(openSettings:)), ","), (None, None, ""), (Some("Quit Moo"), Some(sel!(quitMoo:)), "q")];
    for (title, action, key) in entries {
        let Some(title) = title else {
            menu.addItem(&NSMenuItem::separatorItem(mtm));
            continue;
        };
        let mi = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(NSMenuItem::alloc(mtm), &NSString::from_str(title), action, &NSString::from_str(key))
        };
        unsafe { mi.setTarget(Some(&target)) };
        menu.addItem(&mi);
    }
    STATUS.with(|s| *s.borrow_mut() = Some(StatusItem { item, menu, _target: target }));
}

fn set_status_tooltip(button: &NSView, hotkey: &str) {
    let tip = if hotkey.is_empty() { "Moo".to_string() } else { format!("Moo  ({hotkey})") };
    button.setToolTip(Some(&NSString::from_str(&tip)));
}

/// Register SF Symbol `symbol` as a named image (once) and return the name.
pub fn symbol_icon(symbol: &str) -> String {
    let name = format!("moo-symbol-{symbol}");
    let ns_name = NSString::from_str(&name);
    if NSImage::imageNamed(&ns_name).is_none() {
        let desc = NSString::from_str(symbol);
        match NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str(symbol), Some(&desc)) {
            Some(img) => {
                img.setName(Some(&ns_name));
            }
            None => return String::new(),
        }
    }
    name
}

/// Icons are registered as named images so `<image src={name}>` can find them. Names live in a
/// fixed ring, so file results cannot grow the registry without bound.
pub fn icon_name(path: &str) -> String {
    if let Some(n) = ICONS.with(|m| m.borrow().get(path).cloned()) {
        return n;
    }
    let slot = ICON_RING.with(|r| {
        let mut r = r.borrow_mut();
        let slot = r.0 % ICON_SLOTS;
        r.0 += 1;
        if slot < r.1.len() {
            let evicted = std::mem::replace(&mut r.1[slot], path.to_string());
            ICONS.with(|m| m.borrow_mut().remove(&evicted));
        } else {
            r.1.push(path.to_string());
        }
        slot
    });
    let name = format!("moo-icon-{slot}");
    let ns_name = NSString::from_str(&name);
    if let Some(old) = NSImage::imageNamed(&ns_name) {
        old.setName(None);
    }
    let img = NSWorkspace::sharedWorkspace().iconForFile(&NSString::from_str(path));
    img.setName(Some(&ns_name));
    ICONS.with(|m| m.borrow_mut().insert(path.to_string(), name.clone()));
    warm_later(img);
    name
}

thread_local! {
    static WARM_QUEUE: RefCell<std::collections::VecDeque<Retained<NSImage>>> = RefCell::new(std::collections::VecDeque::new());
    static WARMING: Cell<bool> = const { Cell::new(false) };
}

/// Workspace icons load lazily: an image view draws a placeholder and is not told when the real
/// icon arrives. Drawing each icon once offscreen loads it (about 15 ms each), so that happens one
/// icon per main-loop turn, keeping keys and drawing responsive in between. When the queue runs
/// dry with the panel open, `onKey("icons")` asks the view to redraw.
fn warm_later(img: Retained<NSImage>) {
    WARM_QUEUE.with(|q| q.borrow_mut().push_back(img));
    if !WARMING.with(|w| w.replace(true)) {
        schedule_warm();
    }
}

fn schedule_warm() {
    let when = DispatchTime::try_from(std::time::Duration::from_millis(1)).unwrap_or(DispatchTime::NOW);
    let _ = DispatchQueue::main().after(when, warm_next);
}

fn warm_next() {
    let Some(img) = WARM_QUEUE.with(|q| q.borrow_mut().pop_front()) else {
        WARMING.with(|w| w.set(false));
        // Views drawn before their icon loaded still show the placeholder until redrawn.
        if SHOWN.with(|s| s.get()) {
            defer_callback("key", "icons");
        }
        return;
    };
    let side = NSSize::new(128.0, 128.0);
    let canvas = NSImage::initWithSize(<NSImage as objc2::AllocAnyThread>::alloc(), side);
    #[allow(deprecated)]
    unsafe {
        canvas.lockFocus();
        img.drawInRect(NSRect::new(NSPoint::new(0.0, 0.0), side));
        canvas.unlockFocus();
    }
    schedule_warm();
}

/// Register every app's icon now, A–Z, so they are loaded before Applications is opened.
pub fn warm_app_icons() {
    for p in index::paths() {
        icon_name(&p);
    }
}

// ── Sources: Spotlight files and live app index ─────────────────────────────

pub fn search_files(query: &str, limit: usize, cb: Option<Value>) -> u64 {
    ON_FILES.with(|c| *c.borrow_mut() = cb);
    files::request(query, limit, deliver_files)
}

fn deliver_files(d: files::Delivery) {
    if d.generation != files::latest_generation() {
        return;
    }
    debug_log(&format!("files {:?}: {} hits in {:.1} ms", d.query, d.hits.len(), d.ms));
    let Some(Value::Function(f)) = ON_FILES.with(|c| c.borrow().clone()) else { return };
    let rows: Vec<Value> = d
        .hits
        .iter()
        .map(|h| {
            crate::obj(vec![
                ("name", Value::String(h.name.as_str().into())),
                ("path", Value::String(h.path.as_str().into())),
                ("icon", Value::String(icon_name(&h.path).as_str().into())),
                ("kind", Value::String(if h.is_dir { "Folder" } else { "File" }.into())),
                ("detail", Value::String(h.detail.as_str().into())),
                ("score", Value::Number(h.score as f64)),
            ])
        })
        .collect();
    let payload = crate::obj(vec![
        ("query", Value::String(d.query.as_str().into())),
        ("results", Value::Array(VmRef::new(rows))),
        ("ms", Value::Number(d.ms)),
    ]);
    with_ui(|| {
        let _ = f.call(&[payload]);
    });
}

pub fn watch_apps() -> bool {
    watch::watch(&index::root_strings(), || {
        let (n, ms) = index::reindex();
        warm_app_icons();
        debug_log(&format!("apps reindexed: {n} in {ms:.1} ms"));
    })
}
