//! Windows implementation.

mod control;
mod desktop;
mod insert;
mod shell;

pub use control::{ControlListener, ControlRequest, control_request};
pub use desktop::{CANCEL_SHORTCUT, Desktop, RECORD_SHORTCUT};
pub use insert::{Inserted, Window, copy, focused_window, insert};
pub use shell::{autostart_enabled, disable_autostart, enable_autostart, notify, open};
