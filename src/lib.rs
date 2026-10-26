//! The app's UI, drawn with Native Shell.
//!
//! `main.rs` hosts it in a macOS menu bar popover; `cargo run` starts that
//! host directly.

mod api;
mod cache;
mod calendar;
mod format;
mod model;
pub mod monitor;
mod ui;

pub use ui::{App, set_close_handler, set_open_handler, set_visible};
