//! UI calls track the engine's signal; native menu calls use the same saved preference.
use std::cell::Cell;
use nsh_core::ui::prelude::*;
use crate::settings::{self, Language};
thread_local! { static LANGUAGE: Cell<Option<Signal<Language>>> = const { Cell::new(None) }; }
pub(crate) fn initialize() { LANGUAGE.with(|slot| slot.set(Some(signal(settings::language())))); }
pub(crate) fn refresh() {
    LANGUAGE.with(|slot| { if let Some(signal) = slot.get() { signal.set(settings::language()); } });
}
pub fn language() -> Language {
    LANGUAGE.with(|slot| slot.get().map(|signal| signal.get()).unwrap_or_else(settings::language))
}
pub fn tr(en: &'static str, zh: &'static str) -> &'static str {
    match language() { Language::English => en, Language::SimplifiedChinese => zh }
}
