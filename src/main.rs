//! The macOS host: a status item in the menu bar whose popover shows the
//! Native Shell panel.
//!
//! NSH draws the panel (`codex_reset::App`, a Rust root mounted in an
//! `AppView`); AppKit owns the status item, the popover, opening links and
//! quitting. See "Embedding in a Mac app" in https://nsh.dev/docs/macos.

#![cfg(target_os = "macos")]

mod preferences;
mod updater;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use nsh_platform_macos::{AppOptions, AppView, Root};
use objc2::{
    AnyThread, MainThreadMarker, MainThreadOnly, define_class, msg_send, rc::Retained, runtime::AnyObject,
    runtime::ProtocolObject, sel,
};
use objc2_app_kit::{
    NSAlert, NSBezierPath, NSColor, NSOpenPanel, NSBitmapImageRep, NSModalResponseOK,
    NSAppearance, NSAppearanceNameDarkAqua, NSApplication, NSApplicationActivationPolicy,
    NSBackingStoreType, NSBitmapImageFileType, NSImage, NSPopover, NSPopoverBehavior,
    NSEventMask, NSEventType, NSEventModifierFlags, NSMenu, NSMenuItem,
    NSPopoverDelegate, NSStatusBar, NSStatusItem, NSVariableStatusItemLength, NSViewController,
    NSView, NSWindow, NSWindowStyleMask, NSWorkspace,
};
use objc2_quartz_core::CALayer;
use objc2_foundation::{
    NSArray, NSData, NSDate, NSDefaultRunLoopMode, NSDictionary, NSNotification, NSObject, NSObjectProtocol,
    NSPoint, NSRect, NSRectEdge, NSRunLoop, NSSize, NSString, NSURL, ns_string,
};

/// The popover's size, in points (the panel lays itself out to fill it).
const WIDTH: f64 = 360.0;
const HEIGHT: f64 = 728.0;

struct Menu {
    item: Retained<NSStatusItem>,
    popover: Retained<NSPopover>,
    /// The status item's action target, alive as long as the item.
    _controller: Retained<Controller>,
    /// Kept alive with the popover: dropping it stops the engine.
    _view: Rc<AppView>,
    loaded: Cell<bool>,
    badge: Cell<codex_reset::monitor::Badge>,
}

thread_local! {
    static MENU: RefCell<Option<Menu>> = const { RefCell::new(None) };
    /// When the popover last closed by itself: the click that closed it must
    /// not reopen it in the same breath.
    static CLOSED_AT: Cell<Option<Instant>> = const { Cell::new(None) };
}

fn main() {
    #[cfg(feature = "sparkle-poc")]
    unsafe {
        unsafe extern "C" { fn sparkle_poc_prepare(); }
        sparkle_poc_prepare();
    }
    let arguments: Vec<String> = std::env::args().collect();
    if arguments.iter().any(|argument| argument == "--self-test") {
        self_test();
        return;
    }
    if let Some(flag) = arguments.iter().position(|arg| arg == "--settings-snapshot") {
        let mtm = MainThreadMarker::new().unwrap();
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
        if arguments.iter().any(|arg| arg == "--dark") {
            app.setAppearance(NSAppearance::appearanceNamed(unsafe { NSAppearanceNameDarkAqua }).as_deref());
        }
        updater::start();
        preferences::show();
        pump(0.5);
        preferences::snapshot(arguments.get(flag + 1).expect("snapshot path"));
        return;
    }
    if let Some(flag) = arguments
        .iter()
        .position(|argument| argument == "--snapshot")
    {
        // `--snapshot <path> [seconds]`: render the panel to a PNG and exit,
        // for working on the UI without a screen.
        let path = arguments
            .get(flag + 1)
            .cloned()
            .unwrap_or_else(|| "panel.png".to_owned());
        let wait = arguments
            .get(flag + 2)
            .and_then(|value| value.parse().ok())
            .unwrap_or(4.0);
        let dark = arguments.iter().any(|argument| argument == "--dark");
        // Preview a reminder without touching the real updater or preferences.
        if let Some(index) = arguments.iter().position(|arg| arg == "--update-preview") {
            codex_reset::update_available(arguments.get(index + 1).cloned());
        }
        snapshot(&path, wait, dark);
        return;
    }

    let mtm = MainThreadMarker::new().expect("the menu bar app starts on the main thread");
    let app = NSApplication::sharedApplication(mtm);
    // No Dock icon and no app menu: a menu bar app, not a window app.
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);

    codex_reset::set_open_handler(open_external);
    codex_reset::set_close_handler(close_panel);
    codex_reset::set_update_handler(|| on_main(updater::check));

    MENU.with(|slot| *slot.borrow_mut() = Some(menu_bar(mtm)));
    codex_reset::monitor::start(update_badge);

    // `CODEX_RESET_SHOW_ON_LAUNCH=1` opens the panel with the app, for a
    // screenshot or a demo; a normal launch waits for a click.
    if std::env::var_os("CODEX_RESET_SHOW_ON_LAUNCH").is_some() {
        pump(0.4);
        toggle_panel();
    }

    updater::start();
    if arguments.iter().any(|arg| arg == "--settings") { preferences::show(); }
    #[cfg(feature = "sparkle-poc")]
    unsafe {
        unsafe extern "C" { fn sparkle_poc_start(); }
        sparkle_poc_start();
    }
    app.run();
}

/// The status item, its popover, and the NSH view inside it.
fn menu_bar(mtm: MainThreadMarker) -> Menu {
    // The panel: a Rust root in an AppView, the same engine `nsh run` hosts.
    let view = AppView::new(
        mtm,
        AppOptions {
            app_id: "com.krabs.codex-reset".to_owned(),
            title: "Codex Resets".to_owned(),
            size: (WIDTH as i32, HEIGHT as i32),
            ..AppOptions::default()
        },
    );

    let content = NSViewController::new(mtm);
    content.setView(&view.view());

    // SAFETY: the controller answers `togglePanel:` and the popover delegate
    // call; it lives with the item.
    let target: Retained<Controller> = unsafe { msg_send![Controller::alloc(mtm), init] };

    let popover = NSPopover::new(mtm);
    popover.setBehavior(NSPopoverBehavior::Transient);
    popover.setAnimates(false);
    popover.setContentSize(NSSize::new(WIDTH, HEIGHT));
    // Keep the NSH root inside AppKit's content area. Extending it to the
    // window edges clips the header and footer against the system corners.
    popover.setHasFullSizeContent(false);
    popover.setContentViewController(Some(&content));
    popover.setDelegate(Some(ProtocolObject::from_ref(&*target)));

    // The menu bar item.
    let bar = NSStatusBar::systemStatusBar();
    let item = bar.statusItemWithLength(NSVariableStatusItemLength);
    if let Some(button) = item.button(mtm) {
        match NSImage::initWithData(
            NSImage::alloc(),
            &NSData::with_bytes(&codex_reset::settings::load_avatar()
                .unwrap_or_else(|| include_bytes!("../resources/status-logo.png").to_vec())),
        ) {
            Some(icon) => {
                icon.setSize(NSSize::new(18.0, 18.0));
                icon.setTemplate(false);
                button.setImage(Some(&icon));
            }
            None => button.setTitle(ns_string!("CR")),
        }
        button.sendActionOn(NSEventMask::LeftMouseUp | NSEventMask::RightMouseUp);
        unsafe {
            button.setTarget(Some(&target));
            button.setAction(Some(sel!(togglePanel:)));
        }
    }

    Menu {
        item,
        popover,
        _controller: target,
        _view: view,
        loaded: Cell::new(false),
        badge: Cell::new(codex_reset::monitor::Badge::None),
    }
}

fn is_context_click(kind: NSEventType, modifiers: NSEventModifierFlags) -> bool {
    kind == NSEventType::RightMouseUp
        || (kind == NSEventType::LeftMouseUp && modifiers.contains(NSEventModifierFlags::Control))
}

/// A native secondary-click menu; the primary click continues to use the popover.
fn show_context_menu(target: &Controller) {
    let mtm = target.mtm();
    // Menu tracking runs a nested event loop. Do not hold a MENU borrow across it.
    let Some((button, popover)) = MENU.with(|slot| {
        let slot = slot.borrow();
        let host = slot.as_ref()?;
        Some((host.item.button(mtm)?, host.popover.clone()))
    }) else { return };
    unsafe { popover.performClose(None) };

    let menu = NSMenu::new(mtm);
    menu.setAutoenablesItems(false);
    for label in [
        format!("Codex Resets {}", env!("CARGO_PKG_VERSION")),
        codex_reset::i18n::tr("Auto-refresh: every 60 seconds", "自动刷新：每 60 秒").to_owned(),
    ] {
        let info = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm), &NSString::from_str(&label), None, ns_string!(""),
            )
        };
        info.setEnabled(false);
        menu.addItem(&info);
    }
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    let settings = unsafe {
        let item = NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str(codex_reset::i18n::tr("Settings", "设置")),
            Some(sel!(openSettings:)), ns_string!(","),
        );
        item.setTarget(Some(target));
        item
    };
    menu.addItem(&settings);
    let updates = unsafe {
        let item = NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str(&updater::menu_title()),
            Some(sel!(checkForUpdates:)), ns_string!(""),
        );
        item.setTarget(Some(target));
        item
    };
    updates.setEnabled(updater::can_check());
    menu.addItem(&updates);
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    // The controller implements quitApp: and lives for the entire status item's lifetime.
    let quit = unsafe {
        let item = NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm), &NSString::from_str(codex_reset::i18n::tr("Quit Codex Resets", "退出 Codex Resets")),
            Some(sel!(quitApp:)), ns_string!("q"),
        );
        item.setTarget(Some(target));
        item
    };
    menu.addItem(&quit);
    menu.popUpMenuPositioningItem_atLocation_inView(
        None, NSPoint::new(0.0, 0.0), Some(&button),
    );
}

/// Keep image updates on AppKit's main thread; badge variants have identical sizes.
fn update_badge(badge: codex_reset::monitor::Badge) {
    dispatch2::DispatchQueue::main().exec_async(move || {
        let Some(mtm) = MainThreadMarker::new() else { return };
        MENU.with(|slot| {
            let slot = slot.borrow();
            let Some(menu) = slot.as_ref() else { return };
            let Some(button) = menu.item.button(mtm) else { return };
            menu.badge.set(badge);
            use codex_reset::monitor::Badge;
            let bytes: &[u8] = match badge {
                Badge::None => include_bytes!("../resources/status-logo.png"),
                Badge::NewReset => include_bytes!("../resources/status-logo-new.png"),
                Badge::Scheduled => include_bytes!("../resources/status-logo-scheduled.png"),
            };
            let custom = codex_reset::settings::load_avatar();
            if let Some(image) = NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(custom.as_deref().unwrap_or(bytes))) {
                let image = if custom.is_some() { avatar_badge(&image, badge) } else { image };
                let image = if updater::version().is_some() { update_arrow(&image) } else { image };
                image.setSize(NSSize::new(18.0, 18.0));
                image.setTemplate(false);
                button.setImage(Some(&image));
            }
            if std::env::var_os("CODEX_RESET_TRACE").is_some() {
                eprintln!("Menu badge: {badge:?}");
            }
        });
    });
}

/// Sparkle delivers changes on the main thread; redraw without replacing reset state.
#[cfg(feature = "sparkle")]
fn refresh_update_badge() {
    let badge = MENU.with(|slot| slot.borrow().as_ref().map(|menu| menu.badge.get()));
    if let Some(badge) = badge { update_badge(badge); }
}

/// Upper-left arrow leaves the lower-right reset indicator visible.
#[allow(deprecated)]
fn update_arrow(source: &NSImage) -> Retained<NSImage> {
    let image = NSImage::initWithSize(NSImage::alloc(), NSSize::new(18.0, 18.0));
    image.lockFocus();
    source.drawInRect(NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(18.0, 18.0)));
    NSColor::colorWithSRGBRed_green_blue_alpha(0.12, 0.43, 0.91, 1.0).setFill();
    NSBezierPath::bezierPathWithOvalInRect(NSRect::new(NSPoint::new(0.0, 9.0), NSSize::new(9.0, 9.0))).fill();
    NSColor::whiteColor().setStroke();
    let arrow = NSBezierPath::bezierPath();
    arrow.setLineWidth(1.25);
    arrow.moveToPoint(NSPoint::new(4.5, 10.8));
    arrow.lineToPoint(NSPoint::new(4.5, 15.8));
    arrow.moveToPoint(NSPoint::new(2.3, 13.6));
    arrow.lineToPoint(NSPoint::new(4.5, 15.8));
    arrow.lineToPoint(NSPoint::new(6.7, 13.6));
    arrow.stroke();
    image.unlockFocus();
    image
}

/// Import a local image once; both surfaces use the same normalized PNG.
fn change_avatar(reset: bool) {
    on_main(move || {
        let Some(mtm) = MainThreadMarker::new() else { return };
        let result = (|| -> Result<Option<Vec<u8>>, String> {
            use codex_reset::i18n::tr;
            let path = codex_reset::settings::avatar_path()
                .ok_or_else(|| tr("Cannot locate the settings folder.", "无法找到设置文件夹。").to_owned())?;
            if reset {
                match std::fs::remove_file(&path) {
                    Ok(()) => {},
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
                    Err(_) => return Err(tr("Could not reset the avatar.", "无法重置头像。").to_owned()),
                }
                return Ok(None);
            }
            // Close the transient popover before entering AppKit's modal loop.
            MENU.with(|slot| {
                if let Some(menu) = slot.borrow().as_ref() {
                    unsafe { menu.popover.performClose(None) };
                }
            });
            let panel = NSOpenPanel::openPanel(mtm);
            panel.setCanChooseDirectories(false);
            panel.setCanChooseFiles(true);
            panel.setAllowsMultipleSelection(false);
            panel.setTitle(Some(&NSString::from_str(tr("Choose an avatar", "选择头像"))));
            panel.setPrompt(Some(&NSString::from_str(tr("Choose", "选择"))));
            let types = NSArray::from_retained_slice(&["png", "jpg", "jpeg", "heic", "tiff", "webp"].map(NSString::from_str));
            #[allow(deprecated)]
            panel.setAllowedFileTypes(Some(&types));
            NSApplication::sharedApplication(mtm).activate();
            if panel.runModal() != NSModalResponseOK { return Ok(codex_reset::settings::load_avatar()); }
            let source = panel.URL().and_then(|url| url.path())
                .ok_or_else(|| tr("Could not read this image.", "无法读取此图片。").to_owned())?;
            let source = std::path::PathBuf::from(source.to_string());
            let invalid = || tr("Choose a valid image smaller than 20 MB.", "请选择小于 20 MB 的有效图片。").to_owned();
            if std::fs::metadata(&source).map_err(|_| invalid())?.len() > 20 * 1024 * 1024 { return Err(invalid()); }
            let bytes = std::fs::read(source).map_err(|_| invalid())?;
            let png = normalize_avatar(&bytes).ok_or_else(invalid)?;
            let directory = path.parent().ok_or_else(invalid)?;
            std::fs::create_dir_all(directory).and_then(|_| {
                let temporary = path.with_extension("png.tmp");
                std::fs::write(&temporary, &png)?;
                std::fs::rename(temporary, &path)
            }).map_err(|_| tr("Could not save the avatar.", "无法保存头像。").to_owned())?;
            Ok(Some(png))
        })();
        match result {
            Ok(bytes) => {
                codex_reset::avatar_changed(bytes);
                preferences::refresh();
                let badge = MENU.with(|slot| slot.borrow().as_ref().map(|menu| menu.badge.get()));
                if let Some(badge) = badge { update_badge(badge); }
            }
            Err(message) => {
                let alert = NSAlert::new(mtm);
                alert.setMessageText(&NSString::from_str(&message));
                alert.runModal();
            }
        }
    });
}

#[allow(deprecated)]
fn normalize_avatar(bytes: &[u8]) -> Option<Vec<u8>> {
    let source = NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(bytes))?;
    let size = source.size();
    if !size.width.is_finite() || !size.height.is_finite() || size.width <= 0.0 || size.height <= 0.0 { return None; }
    let edge = 128.0;
    let scale = edge / size.width.min(size.height);
    let image = NSImage::initWithSize(NSImage::alloc(), NSSize::new(edge, edge));
    image.lockFocus();
    NSBezierPath::bezierPathWithOvalInRect(NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(edge, edge))).addClip();
    source.drawInRect(NSRect::new(
        NSPoint::new((edge - size.width * scale) / 2.0, (edge - size.height * scale) / 2.0),
        NSSize::new(size.width * scale, size.height * scale),
    ));
    image.unlockFocus();
    let tiff = image.TIFFRepresentation()?;
    let bitmap = NSBitmapImageRep::initWithData(NSBitmapImageRep::alloc(), &tiff)?;
    unsafe { bitmap.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new()) }.map(|data| data.to_vec())
}

/// Preserve new-reset/scheduled indicators when a custom avatar is installed.
#[allow(deprecated)]
fn avatar_badge(source: &NSImage, badge: codex_reset::monitor::Badge) -> Retained<NSImage> {
    use codex_reset::monitor::Badge;
    let image = NSImage::initWithSize(NSImage::alloc(), NSSize::new(18.0, 18.0));
    image.lockFocus();
    source.drawInRect(NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(18.0, 18.0)));
    if badge != Badge::None {
        let (red, green, blue) = if badge == Badge::NewReset { (0.2, 0.8, 0.4) } else { (1.0, 0.65, 0.1) };
        NSColor::colorWithSRGBRed_green_blue_alpha(red, green, blue, 1.0).setFill();
        NSBezierPath::bezierPathWithOvalInRect(NSRect::new(NSPoint::new(12.0, 0.0), NSSize::new(6.0, 6.0))).fill();
    }
    image.unlockFocus();
    image
}

/// Shows the panel under the status item, or hides it when it is open.
fn toggle_panel() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    MENU.with(|slot| {
        let slot = slot.borrow();
        let Some(menu) = slot.as_ref() else { return };

        if menu.popover.isShown() {
            // SAFETY: `nil` sender is what AppKit's own close actions use.
            unsafe { menu.popover.performClose(None) };
            return;
        }
        // A transient popover closes on the click outside it, which may be
        // this very click on the status item: then it stays closed.
        if CLOSED_AT.with(|closed| {
            closed
                .get()
                .is_some_and(|at| at.elapsed() < Duration::from_millis(250))
        }) {
            return;
        }
        CLOSED_AT.with(|closed| closed.set(None));

        let Some(button) = menu.item.button(mtm) else {
            return;
        };
        // An accessory app is not active: activate it so the panel takes
        // clicks and Escape the way a window does.
        NSApplication::sharedApplication(mtm).activate();
        menu.popover.showRelativeToRect_ofView_preferredEdge(
            button.bounds(),
            &button,
            NSRectEdge::MaxY,
        );
        // Mount only after AppKit attaches and shows the root. Static text
        // mounted while detached can otherwise remain in NSH's deferred queue.
        if !menu.loaded.replace(true) {
            menu._view.load(Root::of::<codex_reset::App>().into());
        }
        // An offscreen positioning rect suppresses the arrow while keeping
        // the transient popover and its system-managed content margins.
        let mut anchor = menu.popover.positioningRect();
        anchor.origin.y += 10000.0;
        menu.popover.setPositioningRect(anchor);
        // Resolve placement before returning to the run loop. Deferring this
        // exposes AppKit's centered position for a frame before the move.
        align_panel();

    });
}

fn align_panel() {
    let Some(mtm) = MainThreadMarker::new() else { return };
    MENU.with(|slot| {
        let slot = slot.borrow();
        let Some(menu) = slot.as_ref() else { return };
        if !menu.popover.isShown() { return; }
        let Some(button) = menu.item.button(mtm) else { return };
        // Use menu-style leading alignment, constrained to the display that
        // owns this status item. Align the visible body, excluding the
        // transparent popover margin, to the bottom of the menu bar.
        if let (Some(status_window), Some(panel_window)) =
            (button.window(), menu._view.view().window())
        {
            if let Some(screen) = status_window.screen() {
                let button_rect = status_window.convertRectToScreen(
                    button.convertRect_toView(button.bounds(), None),
                );
                let frame = panel_window.frame();
                let visible = screen.visibleFrame();
                let x = menu_origin_x(button_rect.origin.x, frame.size.width, visible);
                let panel = menu._view.view();
                let content_rect = panel.convertRect_toView(panel.bounds(), None);
                let y = status_window.frame().origin.y
                    - content_rect.origin.y - content_rect.size.height;
                panel_window.setFrameOrigin(NSPoint::new(x, y));
                // Reopening a popover can discard backing images without a
                // reactive data change. NSH's text layers must draw again.
                if let Some(layer) = panel.layer() {
                    redraw_content_layers(&layer);
                }
            }
        }
    });
}

/// NSH draws text/canvas through CALayer delegates, not NSView.drawRect.
/// Invalidating NSView alone therefore does not restore these backing images.
fn content_layers(layer: &CALayer, layers: &mut Vec<Retained<CALayer>>) {
    if layer.delegate().is_some_and(|delegate| {
        delegate.respondsToSelector(sel!(drawLayer:inContext:))
            && !delegate.respondsToSelector(sel!(displayLayer:))
    }) {
        layers.push(Retained::from(layer));
    }
    // SAFETY: read and traverse the layer tree only on the main thread.
    if let Some(children) = unsafe { layer.sublayers() } {
        for child in children { content_layers(&child, layers); }
    }
}

fn redraw_content_layers(layer: &CALayer) {
    let mut layers = Vec::new();
    content_layers(layer, &mut layers);
    for layer in layers {
        layer.setNeedsDisplay();
        layer.displayIfNeeded();
    }
}

/// Keep a leading-aligned menu inside the active screen, including displays
/// whose coordinates are negative. The small inset leaves room for its edge.
fn menu_origin_x(anchor_x: f64, width: f64, screen: NSRect) -> f64 {
    let left = screen.origin.x + 6.0;
    let right = (screen.origin.x + screen.size.width - width - 6.0).max(left);
    anchor_x.clamp(left, right)
}

/// Opens the panel without a click and reports what AppKit did: the status
/// item's frame, whether the popover shows, the size its view got. Exits
/// non-zero when something is missing, so it can run in CI.
fn self_test() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    codex_reset::set_open_handler(open_external);
    codex_reset::set_close_handler(close_panel);
    MENU.with(|slot| *slot.borrow_mut() = Some(menu_bar(mtm)));

    // Allow macOS to finish rearranging newly inserted status items.
    pump(1.5);
    let panel_frame = || MENU.with(|slot| {
        slot.borrow().as_ref().and_then(|menu| {
            menu._view.view().window().map(|window| window.frame())
        })
    });
    toggle_panel();
    let initial_frame = panel_frame();
    pump(2.0);

    let mut ok = initial_frame.is_some() && initial_frame == panel_frame();
    println!("Initial placement stable across run loop: {ok}");
    let avatar_ok = normalize_avatar(include_bytes!("../resources/codex-resets-logo.jpg"))
        .and_then(|bytes| NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(&bytes)))
        .is_some_and(|image| {
            let size = image.size();
            size.width == size.height && size.width > 0.0
                && [codex_reset::monitor::Badge::None, codex_reset::monitor::Badge::NewReset,
                    codex_reset::monitor::Badge::Scheduled].into_iter()
                    .all(|badge| avatar_badge(&image, badge).size() == NSSize::new(18.0, 18.0))
        }) && normalize_avatar(b"not an image").is_none();
    println!("Avatar import and badge rendering: {avatar_ok}");
    ok &= avatar_ok;

    if std::env::args().any(|arg| arg == "--redraw-test") {
        for _ in 0..3 {
            let layers = MENU.with(|slot| {
                let mut layers = Vec::new();
                if let Some(menu) = slot.borrow().as_ref() {
                    if let Some(root) = menu._view.view().layer() {
                        content_layers(&root, &mut layers);
                    }
                }
                // Only backing images present before hiding must be restored;
                // empty/offscreen views need not allocate an image.
                layers.retain(|layer| unsafe { layer.contents().is_some() });
                layers
            });
            toggle_panel();
            pump(0.3);
            // Reproduce discarded backing images while the popover is hidden.
            for layer in &layers { unsafe { layer.setContents(None) }; }
            toggle_panel();
            let reopened_frame = panel_frame();
            pump(0.5);
            let stable = reopened_frame.is_some() && reopened_frame == panel_frame();
            println!("Reopened placement stable across run loop: {stable}");
            ok &= stable;
            let restored = layers.iter().filter(|layer| unsafe { layer.contents().is_some() }).count();
            println!("Reopened content layers: {restored}/{}", layers.len());
            if layers.is_empty() || restored != layers.len() { ok = false; }
        }
    }
    MENU.with(|slot| {
        let slot = slot.borrow();
        let Some(menu) = slot.as_ref() else {
            println!("FAIL no menu");
            ok = false;
            return;
        };
        match menu.item.button(mtm) {
            Some(button) => {
                let window = button.window();
                println!(
                    "status button: frame {:?} window {:?}",
                    button.frame(),
                    window.as_ref().map(|window| window.frame())
                );
                if window.is_none() {
                    ok = false;
                }
            }
            None => {
                println!("FAIL no status button");
                ok = false;
            }
        }
        println!("popover isShown: {}", menu.popover.isShown());
        println!("popover content size: {:?}", menu.popover.contentSize());
        // The NSH root does not lay out against AppKit's safe-area guide.
        // Full-size content would put the header/footer inside the clip mask.
        if menu.popover.hasFullSizeContent() {
            println!("FAIL popover content extends into the system clip region");
            ok = false;
        }
        let panel = menu._view.view();
        println!("panel frame: {:?} bounds: {:?} safe area: {:?}", panel.frame(), panel.bounds(), panel.safeAreaRect());
        if let Some(window) = panel.window() {
            println!("window frame: {:?}", window.frame());
            if let Some(button) = menu.item.button(mtm) {
                if let Some(status_window) = button.window() {
                    if let Some(screen) = status_window.screen() {
                        let anchor = status_window.convertRectToScreen(
                            button.convertRect_toView(button.bounds(), None),
                        );
                        let expected = menu_origin_x(anchor.origin.x, window.frame().size.width, screen.visibleFrame());
                        let content_rect = panel.convertRect_toView(panel.bounds(), None);
                        let content_top = window.frame().origin.y + content_rect.origin.y + content_rect.size.height;
                        if (content_top - status_window.frame().origin.y).abs() > 1.0 {
                            println!("FAIL panel is not flush with the menu bar");
                            ok = false;
                        }
                        if (window.frame().origin.x - expected).abs() > 4.0 {
                            println!("FAIL menu horizontal alignment: expected {expected}");
                            ok = false;
                        }
                    }
                }
            }
            if let Some(content) = window.contentView() {
                println!(
                    "window content bounds: {:?} frame: {:?}",
                    content.bounds(),
                    content.frame()
                );
                render_view(&content, "/tmp/popover-render.png");
                // SAFETY: only read the parent on the main thread while the window is alive.
                if let Some(frame_view) = unsafe { content.superview() } {
                    render_view(&frame_view, "/tmp/popover-frame.png");
                }
            }
        }
        if !menu.popover.isShown() {
            ok = false;
        }
    });

    println!(
        "{}",
        if ok {
            "SELF-TEST OK"
        } else {
            "SELF-TEST FAILED"
        }
    );

    if std::env::args().any(|argument| argument == "--quit") {
        // NSApp.terminate should end the process here.
        quit_now();
        pump(2.0);
        println!("SELF-TEST FAILED to quit");
        std::process::exit(1);
    }

    std::process::exit(if ok { 0 } else { 1 });
}

/// Draws the panel offscreen into a PNG (a development aid: the menu bar is
/// not capturable without screen-recording permission).
fn snapshot(path: &str, wait: f64, dark: bool) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Prohibited);
    if dark {
        // SAFETY: NSAppearanceNameDarkAqua is AppKit's constant.
        if let Some(appearance) = NSAppearance::appearanceNamed(unsafe { NSAppearanceNameDarkAqua })
        {
            app.setAppearance(Some(&appearance));
        }
    }

    let view = AppView::new(
        mtm,
        AppOptions {
            app_id: "com.krabs.codex-reset".to_owned(),
            title: "Codex Resets".to_owned(),
            size: (WIDTH as i32, HEIGHT as i32),
            ..AppOptions::default()
        },
    );

    let frame = NSRect::new(NSPoint::new(-6000.0, -6000.0), NSSize::new(WIDTH, HEIGHT));
    // SAFETY: NSWindow's designated initializer, on the main thread.
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            frame,
            NSWindowStyleMask::Borderless,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    window.setContentView(Some(&view.view()));
    window.orderFront(None);
    view.load(Root::of::<codex_reset::App>().into());
    codex_reset::set_visible(true);

    pump(wait);
    render_view(&view.view(), path);
    std::process::exit(0);
}

/// Writes `view` as a PNG at `path`.
fn render_view(view: &NSView, path: &str) {
    let bounds = view.bounds();
    if let Some(bitmap) = view.bitmapImageRepForCachingDisplayInRect(bounds) {
        view.cacheDisplayInRect_toBitmapImageRep(bounds, &bitmap);
        // SAFETY: PNG encodes an NSBitmapImageRep with an empty properties
        // dictionary; the data is then written to `path`.
        if let Some(data) = unsafe {
            bitmap.representationUsingType_properties(
                NSBitmapImageFileType::PNG,
                &NSDictionary::new(),
            )
        } {
            data.writeToFile_atomically(&NSString::from_str(path), true);
        }
    }
}

/// Runs the main run loop for `seconds`, so the engine commits frames and the
/// network answers arrive.
fn pump(seconds: f64) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs_f64(seconds);
    let run_loop = NSRunLoop::currentRunLoop();
    while std::time::Instant::now() < deadline {
        let until = NSDate::dateWithTimeIntervalSinceNow(0.02);
        // SAFETY: NSDefaultRunLoopMode is AppKit's constant.
        run_loop.runMode_beforeDate(unsafe { NSDefaultRunLoopMode }, &until);
    }
}

/// Runs `work` on the main thread: the panel's handlers run on the engine
/// thread, and AppKit answers only on the main one.
fn on_main(work: impl FnOnce() + Send + 'static) {
    dispatch2::DispatchQueue::main().exec_async(work);
}

/// Hides the panel: a chosen link leaves the menu bar.
fn close_panel() {
    on_main(|| {
        MENU.with(|slot| {
            if let Some(menu) = slot.borrow().as_ref() {
                // SAFETY: `nil` sender is what AppKit's own close actions use.
                unsafe { menu.popover.performClose(None) };
            }
        });
    });
}

/// Ends the app (there is no window to close instead).
fn quit_now() {
    on_main(|| {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        NSApplication::sharedApplication(mtm).terminate(None);
    });
}

/// Opens a link in the default browser (the panel's rows and its credit).
fn open_external(url: &str) {
    let Some(url) = NSURL::URLWithString(&NSString::from_str(url)) else {
        return;
    };
    NSWorkspace::sharedWorkspace().openURL(&url);
}

define_class!(
    /// The status item's action target.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "CodexResetController"]
    struct Controller;

    unsafe impl NSObjectProtocol for Controller {}

    impl Controller {
        #[unsafe(method(togglePanel:))]
        fn toggle_panel_action(&self, _sender: Option<&AnyObject>) {
            let event = NSApplication::sharedApplication(self.mtm()).currentEvent();
            if event.is_some_and(|event| is_context_click(event.r#type(), event.modifierFlags())) {
                show_context_menu(self);
            } else {
                toggle_panel();
            }
        }

        #[unsafe(method(openSettings:))]
        fn settings_action(&self, _sender: Option<&AnyObject>) {
            // Wait until menu tracking ends before opening the settings window.
            on_main(|| {
                preferences::show();
            });
        }

        #[unsafe(method(checkForUpdates:))]
        fn check_updates_action(&self, _sender: Option<&AnyObject>) {
            on_main(updater::check);
        }

        #[unsafe(method(changeAutomaticUpdates:))]
        fn automatic_updates_action(&self, sender: &objc2_app_kit::NSButton) {
            updater::set_automatic(sender.state() == 1);
            preferences::refresh();
        }

        #[unsafe(method(changeLanguage:))]
        fn language_action(&self, sender: &objc2_app_kit::NSPopUpButton) {
            preferences::change_language(sender.indexOfSelectedItem());
        }

        #[unsafe(method(changeAvatar:))]
        fn avatar_action(&self, _sender: Option<&AnyObject>) { change_avatar(false); }

        #[unsafe(method(resetAvatar:))]
        fn reset_avatar_action(&self, _sender: Option<&AnyObject>) { change_avatar(true); }

        #[unsafe(method(quitApp:))]
        fn quit_action(&self, _sender: Option<&AnyObject>) {
            quit_now();
        }
    }

    unsafe impl NSPopoverDelegate for Controller {
        #[unsafe(method(popoverDidShow:))]
        fn popover_did_show(&self, _notification: &NSNotification) {
            codex_reset::set_visible(true);
        }

        #[unsafe(method(popoverDidClose:))]
        fn popover_did_close(&self, _notification: &NSNotification) {
            codex_reset::set_visible(false);
            CLOSED_AT.with(|closed| closed.set(Some(Instant::now())));
        }
    }
);

#[cfg(test)]
mod placement_tests {
    use super::*;

    #[test]
    fn secondary_click_opens_menu_without_changing_primary_click() {
        assert!(is_context_click(NSEventType::RightMouseUp, NSEventModifierFlags::empty()));
        assert!(is_context_click(NSEventType::LeftMouseUp, NSEventModifierFlags::Control));
        assert!(!is_context_click(NSEventType::LeftMouseUp, NSEventModifierFlags::empty()));
        assert!(!is_context_click(NSEventType::LeftMouseUp, NSEventModifierFlags::Command));
    }

    #[test]
    fn menu_aligns_to_button_and_stays_on_its_screen() {
        let screen = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1440.0, 900.0));
        assert_eq!(menu_origin_x(600.0, 386.0, screen), 600.0);
        assert_eq!(menu_origin_x(1400.0, 386.0, screen), 1048.0);
        assert_eq!(menu_origin_x(0.0, 386.0, screen), 6.0);
        let secondary = NSRect::new(NSPoint::new(-1440.0, 0.0), screen.size);
        assert_eq!(menu_origin_x(-800.0, 386.0, secondary), -800.0);
        assert_eq!(menu_origin_x(-10.0, 386.0, secondary), -392.0);
    }
}
