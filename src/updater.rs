//! Main-thread access to Sparkle. Source-only builds do not load an updater.
#[cfg(feature = "sparkle")]
mod native {
    unsafe extern "C" {
        pub fn codex_updater_start() -> bool;
        pub fn codex_updater_set_handler(handler: extern "C" fn(*const std::ffi::c_char));
        pub fn codex_updater_version() -> *const std::ffi::c_char;
        pub fn codex_updater_available() -> bool;
        pub fn codex_updater_can_check() -> bool;
        pub fn codex_updater_check();
        pub fn codex_updater_automatic() -> bool;
        pub fn codex_updater_set_automatic(enabled: bool);
    }
}

pub fn start() {
    #[cfg(feature = "sparkle")]
    unsafe {
        native::codex_updater_set_handler(changed);
        native::codex_updater_start();
    }
}
pub fn available() -> bool {
    #[cfg(feature = "sparkle")]
    unsafe {
        return native::codex_updater_available();
    }
    #[cfg(not(feature = "sparkle"))]
    false
}
pub fn can_check() -> bool {
    #[cfg(feature = "sparkle")]
    unsafe {
        return native::codex_updater_can_check();
    }
    #[cfg(not(feature = "sparkle"))]
    false
}
pub fn check() {
    #[cfg(feature = "sparkle")]
    unsafe {
        native::codex_updater_check();
    }
}
pub fn automatic() -> bool {
    #[cfg(feature = "sparkle")]
    unsafe {
        return native::codex_updater_automatic();
    }
    #[cfg(not(feature = "sparkle"))]
    false
}
pub fn set_automatic(enabled: bool) {
    #[cfg(feature = "sparkle")]
    unsafe {
        native::codex_updater_set_automatic(enabled);
    }
    #[cfg(not(feature = "sparkle"))]
    let _ = enabled;
}

#[cfg(feature = "sparkle")]
extern "C" fn changed(value: *const std::ffi::c_char) {
    let version = if value.is_null() { None } else {
        Some(unsafe { std::ffi::CStr::from_ptr(value) }.to_string_lossy().into_owned())
    };
    codex_reset::update_available(version);
    super::refresh_update_badge();
}

pub fn version() -> Option<String> {
    #[cfg(feature = "sparkle")]
    unsafe {
        let value = native::codex_updater_version();
        if !value.is_null() {
            return Some(std::ffi::CStr::from_ptr(value).to_string_lossy().into_owned());
        }
    }
    None
}

pub fn menu_title() -> String {
    match version() {
        Some(version) => format!("{} v{version}…", codex_reset::i18n::tr("Update to", "更新至")),
        None => codex_reset::i18n::tr("Check for Updates…", "检查更新…").to_owned(),
    }
}
