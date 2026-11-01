//! Windows implementation.

mod hotkey;
mod insert;

pub use hotkey::{CANCEL_SHORTCUT, Hotkeys, RECORD_SHORTCUT};
pub use insert::{Inserted, Window, focused_window, insert};
