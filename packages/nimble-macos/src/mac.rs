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
use dispatch2::DispatchQueue;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{sel, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAppearanceCustomization, NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSEvent, NSEventMask,
    NSEventModifierFlags, NSPanel, NSScreen, NSTextField, NSView, NSWindow, NSWindowButton,
    NSWindowCollectionBehavior,
    NSWindowDidBecomeKeyNotification, NSWindowDidResignKeyNotification, NSWindowStyleMask,
    NSImage, NSPasteboard, NSPasteboardTypeString, NSWindowTitleVisibility, NSWorkspace,
};
use objc2_foundation::{
    NSNotification, NSNotificationCenter, NSPoint, NSRect, NSSize, NSString, NSURL,
};
use tishlang_core::{Value, VmRef};

use crate::{files, index, watch};

const ICON_SLOTS: usize = 512;
use tishlang_ui::runtime::{run_with_current_root, LEGACY_ROOT_ID};

thread_local! {
    static ON_KEY: RefCell<Option<Value>> = const { RefCell::new(None) };
    static ON_SHOW: RefCell<Option<Value>> = const { RefCell::new(None) };
    static PENDING: RefCell<Vec<(&'static str, &'static str)>> = const { RefCell::new(Vec::new()) };
    static MONITOR: RefCell<Option<Retained<AnyObject>>> = const { RefCell::new(None) };
    static PANEL_SIZE: Cell<(f64, f64)> = const { Cell::new((720.0, 440.0)) };
    static ICONS: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
    static HOTKEY_HANDLER: Cell<bool> = const { Cell::new(false) };
    static SHOWN: Cell<bool> = const { Cell::new(false) };
    static PANEL: RefCell<Option<Retained<NSWindow>>> = const { RefCell::new(None) };
    static ON_FILES: RefCell<Option<Value>> = const { RefCell::new(None) };
    static ICON_RING: RefCell<(usize, Vec<String>)> = const { RefCell::new((0, Vec::new())) };
}

pub fn set_callbacks(on_key: Option<Value>, on_show: Option<Value>) {
    ON_KEY.with(|c| *c.borrow_mut() = on_key);
    ON_SHOW.with(|c| *c.borrow_mut() = on_show);
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

/// Queue `(callback, arg)` and flush it from the main queue once the current handler returns.
fn defer_callback(which: &'static str, arg: &'static str) {
    PENDING.with(|q| q.borrow_mut().push((which, arg)));
    DispatchQueue::main().exec_async(flush_pending);
}

fn flush_pending() {
    let events = PENDING.with(|q| std::mem::take(&mut *q.borrow_mut()));
    if events.is_empty() {
        return;
    }
    run_with_current_root(LEGACY_ROOT_ID, || {
        for (which, arg) in events {
            let cb = match which {
                "key" => ON_KEY.with(|c| c.borrow().clone()),
                _ => ON_SHOW.with(|c| c.borrow().clone()),
            };
            if let Some(Value::Function(f)) = cb {
                let _ = f.call(&[Value::String(arg.into())]);
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
    let top = vf.origin.y + vf.size.height * 0.80;
    w.setFrameOrigin(NSPoint::new(x, top - f.size.height));
}

/// macOS 14+ refuses activation requests from background apps, so the launcher must take keys
/// without activating, and only an NSPanel honours `NonactivatingPanel`. tish-macos creates a plain
/// NSWindow (re-classing it breaks AppKit's KVO), so its root view moves into our own panel. The
/// host keeps measuring its window's content view, so that window keeps a same-size placeholder.
fn adopt_into_panel(host_window: &NSWindow, mtm: MainThreadMarker) -> Option<Retained<NSWindow>> {
    let root = host_window.contentView()?;
    let (pw, ph) = PANEL_SIZE.with(|c| c.get());
    let rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(pw, ph));

    let placeholder = NSView::initWithFrame(NSView::alloc(mtm), rect);
    host_window.setContentView(Some(&placeholder));
    host_window.setContentSize(NSSize::new(pw, ph));
    host_window.orderOut(None);

    let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
        NSPanel::alloc(mtm),
        rect,
        NSWindowStyleMask::Titled
            | NSWindowStyleMask::FullSizeContentView
            | NSWindowStyleMask::NonactivatingPanel,
        NSBackingStoreType::Buffered,
        false,
    );
    unsafe { panel.setReleasedWhenClosed(false) };
    panel.setFloatingPanel(true);
    panel.setBecomesKeyOnlyIfNeeded(false);
    panel.setHidesOnDeactivate(false);
    panel.setAppearance(host_window.appearance().as_deref());
    panel.setBackgroundColor(Some(&host_window.backgroundColor()));
    panel.setContentView(Some(&root));
    Some(Retained::into_super(panel))
}

fn debug_log(msg: &str) {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    if std::env::var_os("NIMBLE_DEBUG").is_some() {
        let t = START.get_or_init(std::time::Instant::now).elapsed().as_secs_f64() * 1000.0;
        eprintln!("nimble: [{t:.1} ms] {msg}");
    }
}

fn hide_on_resign_key(w: &NSWindow) {
    let center = NSNotificationCenter::defaultCenter();
    let resign = RcBlock::new(|_n: NonNull<NSNotification>| {
        debug_log("panel key=false");
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
    w.setContentSize(NSSize::new(pw, ph));
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

fn focus_search(w: &NSWindow) {
    let Some(content) = w.contentView() else { return };
    if let Some(tf) = first_editable_text_field(&content) {
        w.makeFirstResponder(Some(&tf));
        unsafe { tf.selectText(None) };
    }
}

pub fn show() {
    let Some(mtm) = MainThreadMarker::new() else { return };
    if let Some(w) = main_window(mtm) {
        position_panel(&w, mtm);
        w.orderFrontRegardless();
        w.makeKeyWindow();
        focus_search(&w);
    }
    SHOWN.with(|s| s.set(true));
    debug_log("panel ordered front");
    defer_callback("show", "show");
}

pub fn hide() {
    let Some(mtm) = MainThreadMarker::new() else { return };
    SHOWN.with(|s| s.set(false));
    if let Some(w) = main_window(mtm) {
        w.orderOut(None);
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

fn install_key_monitor() {
    if MONITOR.with(|m| m.borrow().is_some()) {
        return;
    }
    let block = RcBlock::new(|ev: NonNull<NSEvent>| -> *mut NSEvent {
        let e = unsafe { ev.as_ref() };
        if edit_shortcut(e) {
            return std::ptr::null_mut();
        }
        let ctrl = e.modifierFlags().contains(NSEventModifierFlags::Control);
        let name = match (e.keyCode(), ctrl) {
            (125, _) | (45, true) => Some("down"),
            (126, _) | (35, true) => Some("up"),
            (36, _) | (76, _) => Some("enter"),
            (53, _) => Some("escape"),
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
    hide_on_resign_key(&w);
    install_key_monitor();
    show();
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
}

const fn fourcc(s: &[u8; 4]) -> u32 {
    ((s[0] as u32) << 24) | ((s[1] as u32) << 16) | ((s[2] as u32) << 8) | (s[3] as u32)
}

extern "C" fn on_hotkey(_next: *mut c_void, _event: *mut c_void, _user: *mut c_void) -> i32 {
    debug_log(&format!(
        "hotkey shown={} key={}",
        SHOWN.with(|s| s.get()),
        panel_has_keys()
    ));
    toggle();
    0
}

fn parse_hotkey(spec: &str) -> Result<(u32, u32), String> {
    let mut mods = 0u32;
    let mut key = None;
    for part in spec.split('+').map(|p| p.trim().to_ascii_lowercase()) {
        match part.as_str() {
            "cmd" | "command" => mods |= 0x0100,
            "shift" => mods |= 0x0200,
            "alt" | "opt" | "option" => mods |= 0x0800,
            "ctrl" | "control" => mods |= 0x1000,
            "space" => key = Some(49),
            "k" => key = Some(40),
            "j" => key = Some(38),
            "n" => key = Some(45),
            "p" => key = Some(35),
            other => return Err(format!("unsupported hotkey part `{other}`")),
        }
    }
    key.map(|k| (k, mods)).ok_or_else(|| format!("hotkey `{spec}` has no key"))
}

pub fn register_hotkey(spec: &str) -> Result<(), String> {
    let (code, mods) = parse_hotkey(spec)?;
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
        let mut out = std::ptr::null_mut();
        let id = EventHotKeyID { signature: fourcc(b"nmbl"), id: 1 };
        let st = RegisterEventHotKey(code, mods, id, target, 0, &mut out);
        if st != 0 {
            return Err(format!("`{spec}` is unavailable (RegisterEventHotKey: {st})"));
        }
    }
    Ok(())
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

pub fn copy_text(text: &str) -> bool {
    let pb = NSPasteboard::generalPasteboard();
    pb.clearContents();
    pb.setString_forType(&NSString::from_str(text), unsafe { NSPasteboardTypeString })
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
    let name = format!("nimble-icon-{slot}");
    let ns_name = NSString::from_str(&name);
    if let Some(old) = NSImage::imageNamed(&ns_name) {
        old.setName(None);
    }
    let img = NSWorkspace::sharedWorkspace().iconForFile(&NSString::from_str(path));
    img.setName(Some(&ns_name));
    ICONS.with(|m| m.borrow_mut().insert(path.to_string(), name.clone()));
    name
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
    run_with_current_root(LEGACY_ROOT_ID, || {
        let _ = f.call(&[payload]);
    });
}

pub fn watch_apps() -> bool {
    watch::watch(&index::root_strings(), || {
        let (n, ms) = index::reindex();
        debug_log(&format!("apps reindexed: {n} in {ms:.1} ms"));
    })
}
