//! A small native preferences window, independent of the status popover.
use super::*;
use codex_reset::{i18n::tr, settings::{self, Language}};
use objc2_app_kit::{NSBox, NSBoxType, NSButton, NSFont, NSImageView, NSPopUpButton,
    NSTextField};

const LANGUAGES: &[(Language, &str)] = &[
    (Language::English, "English"),
    (Language::SimplifiedChinese, "简体中文"),
];

struct Preferences {
    window: Retained<NSWindow>,
    language: Retained<NSPopUpButton>,
    language_label: Retained<NSTextField>,
    avatar_label: Retained<NSTextField>,
    avatar_hint: Retained<NSTextField>,
    saved_hint: Retained<NSTextField>,
    avatar: Retained<NSImageView>,
    change: Retained<NSButton>,
    reset: Retained<NSButton>,
    updates: Retained<NSButton>,
    updates_hint: Retained<NSTextField>,
    _controller: Retained<Controller>,
}

thread_local! {
    static PREFERENCES: RefCell<Option<Preferences>> = const { RefCell::new(None) };
}

fn rect(x: f64, y: f64, width: f64, height: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(width, height))
}

fn label(mtm: MainThreadMarker, frame: NSRect, size: f64, secondary: bool) -> Retained<NSTextField> {
    let label = NSTextField::labelWithString(ns_string!(""), mtm);
    label.setFrame(frame);
    label.setFont(Some(&NSFont::systemFontOfSize(size)));
    if secondary { label.setTextColor(Some(&NSColor::secondaryLabelColor())); }
    label
}

impl Preferences {
    fn new(mtm: MainThreadMarker) -> Self {
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm), rect(0.0, 0.0, 440.0, 324.0),
                NSWindowStyleMask::Titled | NSWindowStyleMask::Closable,
                NSBackingStoreType::Buffered, false,
            )
        };
        // The retained window is reused after closing, including its action target.
        unsafe { window.setReleasedWhenClosed(false) };
        window.center();
        let content = NSBox::initWithFrame(NSBox::alloc(mtm), rect(0.0, 0.0, 440.0, 324.0));
        content.setBoxType(NSBoxType::Custom);
        content.setBorderWidth(0.0);
        content.setFillColor(&NSColor::windowBackgroundColor());
        window.setContentView(Some(&content));
        let group = NSBox::initWithFrame(NSBox::alloc(mtm), rect(20.0, 50.0, 400.0, 254.0));
        group.setBoxType(NSBoxType::Custom);
        group.setCornerRadius(10.0);
        group.setBorderWidth(0.5);
        group.setBorderColor(&NSColor::separatorColor());
        group.setFillColor(&NSColor::controlBackgroundColor());
        content.addSubview(&group);
        let divider = NSBox::initWithFrame(NSBox::alloc(mtm), rect(38.0, 238.0, 364.0, 1.0));
        divider.setBoxType(NSBoxType::Separator);
        content.addSubview(&divider);
        let target: Retained<Controller> = unsafe { msg_send![Controller::alloc(mtm), init] };
        let language = NSPopUpButton::initWithFrame_pullsDown(
            NSPopUpButton::alloc(mtm), rect(194.0, 257.0, 208.0, 30.0), false,
        );
        let titles: Vec<_> = LANGUAGES.iter().map(|(_, name)| NSString::from_str(name)).collect();
        language.addItemsWithTitles(&NSArray::from_retained_slice(&titles));
        unsafe {
            language.setTarget(Some(&target));
            language.setAction(Some(sel!(changeLanguage:)));
        }
        content.addSubview(&language);
        let language_label = label(mtm, rect(38.0, 262.0, 140.0, 20.0), 13.0, false);
        let avatar_label = label(mtm, rect(100.0, 203.0, 280.0, 20.0), 13.0, false);
        let avatar_hint = label(mtm, rect(100.0, 183.0, 290.0, 18.0), 11.0, true);
        let saved_hint = label(mtm, rect(24.0, 18.0, 392.0, 18.0), 11.0, true);
        for field in [&language_label, &avatar_label, &avatar_hint, &saved_hint] { content.addSubview(field); }
        let avatar = NSImageView::initWithFrame(NSImageView::alloc(mtm), rect(38.0, 164.0, 48.0, 48.0));
        content.addSubview(&avatar);
        let change = unsafe {
            NSButton::buttonWithTitle_target_action(ns_string!(""), Some(&target), Some(sel!(changeAvatar:)), mtm)
        };
        change.setFrame(rect(94.0, 144.0, 144.0, 32.0));
        content.addSubview(&change);
        let reset = unsafe {
            NSButton::buttonWithTitle_target_action(ns_string!(""), Some(&target), Some(sel!(resetAvatar:)), mtm)
        };
        reset.setFrame(rect(240.0, 144.0, 156.0, 32.0));
        content.addSubview(&reset);
        let updates_divider = NSBox::initWithFrame(NSBox::alloc(mtm), rect(38.0, 128.0, 364.0, 1.0));
        updates_divider.setBoxType(NSBoxType::Separator);
        content.addSubview(&updates_divider);
        let updates = unsafe {
            NSButton::checkboxWithTitle_target_action(ns_string!(""), Some(&target), Some(sel!(changeAutomaticUpdates:)), mtm)
        };
        updates.setFrame(rect(38.0, 94.0, 364.0, 22.0));
        content.addSubview(&updates);
        let updates_hint = label(mtm, rect(58.0, 70.0, 344.0, 18.0), 11.0, true);
        content.addSubview(&updates_hint);
        Self { window, language, language_label, avatar_label, avatar_hint, saved_hint,
            avatar, change, reset, updates, updates_hint, _controller: target }
    }

    fn refresh(&self) {
        self.window.setTitle(&NSString::from_str(tr("Codex Resets Settings", "Codex Resets 设置")));
        for (field, value) in [
            (&self.language_label, tr("Language", "语言")),
            (&self.avatar_label, tr("Avatar", "头像")),
            (&self.avatar_hint, tr("Menu bar and popover", "显示在菜单栏和弹出面板中")),
            (&self.saved_hint, tr("Changes are saved automatically.", "更改会自动保存。")),
        ] { field.setStringValue(&NSString::from_str(value)); }
        self.updates.setTitle(&NSString::from_str(tr("Automatically check for updates", "自动检查更新")));
        self.updates.setEnabled(updater::available());
        self.updates.setState(if updater::automatic() { 1 } else { 0 });
        self.updates_hint.setStringValue(&NSString::from_str(if updater::available() {
            tr("Daily checks. Badge only; no automatic pop-up.", "每天检查，仅显示更新标记，不自动弹窗。")
        } else {
            tr("Updates are unavailable in this build.", "此构建暂不支持更新。")
        }));
        self.change.setTitle(&NSString::from_str(tr("Choose Image…", "选择图片…")));
        self.reset.setTitle(&NSString::from_str(tr("Restore Default", "恢复默认")));
        if let Some(index) = LANGUAGES.iter().position(|(language, _)| *language == settings::language()) {
            self.language.selectItemAtIndex(index as isize);
        }
        let custom = settings::load_avatar();
        self.reset.setEnabled(custom.is_some());
        let default = normalize_avatar(include_bytes!("../resources/codex-resets-logo.jpg"));
        let bytes = custom.as_deref().or(default.as_deref()).unwrap_or(include_bytes!("../resources/status-logo.png"));
        let image = NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(bytes));
        self.avatar.setImage(image.as_deref());
    }
}

pub fn show() {
    let Some(mtm) = MainThreadMarker::new() else { return };
    PREFERENCES.with(|slot| {
        let mut slot = slot.borrow_mut();
        let preferences = slot.get_or_insert_with(|| Preferences::new(mtm));
        preferences.refresh();
        NSApplication::sharedApplication(mtm).activate();
        preferences.window.makeKeyAndOrderFront(None);
    });
}

pub fn refresh() {
    PREFERENCES.with(|slot| {
        if let Some(preferences) = slot.borrow().as_ref() { preferences.refresh(); }
    });
}

pub fn change_language(index: isize) {
    let Some(&(language, _)) = usize::try_from(index).ok().and_then(|index| LANGUAGES.get(index)) else { return };
    if settings::set_language(language).is_ok() {
        codex_reset::refresh_language();
    } else if let Some(mtm) = MainThreadMarker::new() {
        let alert = NSAlert::new(mtm);
        alert.setMessageText(&NSString::from_str(tr("Could not save settings", "无法保存设置")));
        alert.runModal();
    }
    refresh();
}

pub fn snapshot(path: &str) {
    PREFERENCES.with(|slot| {
        if let Some(preferences) = slot.borrow().as_ref() {
            if let Some(content) = preferences.window.contentView() {
                // Include the system title bar in visual checks.
                if let Some(frame) = unsafe { content.superview() } { render_view(&frame, path); }
            }
        }
    });
}
