//! Main-thread access to Sparkle. Source-only builds do not load an updater.
#[cfg(feature = "sparkle")]
mod native {
    unsafe extern "C" {
        pub fn codex_updater_start() -> bool;
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
