//! Windows implementation.

mod control;
mod desktop;
mod insert;
mod locale;
mod overlay;
mod pipe_owner;
mod shell;

pub use control::{ControlListener, ControlRequest, control_request};
pub use desktop::{CANCEL_SHORTCUT, Desktop, RECORD_SHORTCUT};
pub use insert::{Inserted, Window, copy, focused_window, insert};
pub(crate) use locale::user_ui_language;
pub use overlay::Overlay;
pub use pipe_owner::pipe_server_is_current_user;
pub use shell::{autostart_enabled, disable_autostart, enable_autostart, notify, open};
