//! Windows implementation.

mod control;
mod desktop;
mod insert;
mod locale;
mod overlay;
mod pipe_name;
mod pipe_owner;
mod shell;
mod socket;

pub use control::{ControlListener, ControlRequest, control_request};
pub use desktop::Desktop;
pub use insert::{Inserted, Window, copy, focused_window, insert};
pub(crate) use locale::user_ui_language;
pub use overlay::Overlay;
pub use pipe_name::pipe_name;
pub use pipe_owner::pipe_server_is_trusted;
pub use shell::{
    autostart_enabled, disable_autostart, enable_autostart, notify, open, restrict_to_user,
};
pub use socket::exclusive_address_use;
