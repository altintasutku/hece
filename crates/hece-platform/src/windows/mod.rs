//! Windows implementation.

mod control;
mod hotkey;
mod insert;
mod shell;

pub use control::{ControlListener, ControlRequest, control_request};
pub use hotkey::{CANCEL_SHORTCUT, Hotkeys, RECORD_SHORTCUT};
pub use insert::{Inserted, Window, copy, focused_window, insert};
pub use shell::{autostart_enabled, disable_autostart, enable_autostart, notify};
