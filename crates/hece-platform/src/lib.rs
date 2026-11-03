//! Desktop integration. Windows: global hotkeys (`RegisterHotKey`), insertion as clipboard + one
//! Ctrl+V, the focused window, the daemon control pipe, toasts and autostart. Other platforms
//! have no implementation yet.

mod i18n;
pub mod overlay;

pub use i18n::{Lang, menu_label, tray_status, ui_language};

#[cfg(windows)]
mod windows;

#[cfg(windows)]
pub use windows::{
    ControlListener, ControlRequest, Desktop, Inserted, Overlay, Window, autostart_enabled,
    control_request, copy, disable_autostart, enable_autostart, exclusive_address_use,
    focused_window, insert, notify, open, pipe_name, pipe_server_is_trusted, restrict_to_user,
};

/// Which global shortcut fired.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shortcut {
    /// Start/stop (or hold to talk).
    Record,
    /// Cancel the recording or the latest dictation.
    Cancel,
}

/// A shortcut went down or up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyEvent {
    /// Pressed (repeats while held are not reported).
    Pressed(Shortcut),
    /// Released.
    Released(Shortcut),
}

/// Platform failures. [`PlatformError::code`] is the stable user-facing code.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PlatformError {
    /// Another application already owns the shortcut.
    #[error("the shortcut {0} is already used by another application")]
    HotkeyTaken(String),
    /// Setting up hotkeys failed.
    #[error("global shortcuts could not be set up")]
    Hotkey,
    /// The clipboard could not be used.
    #[error("the clipboard is not available")]
    Clipboard,
    /// The control pipe could not be used.
    #[error("the daemon control pipe failed")]
    Control,
    /// Another daemon is already running.
    #[error("hece is already running")]
    AlreadyRunning,
    /// No daemon is running.
    #[error("hece is not running")]
    NotRunning,
    /// Another account holds the pipe name this user's daemon or engine needs.
    #[error("another user holds hece's pipe; sign that user out or restart Windows")]
    ForeignPipe,
    /// Autostart could not be changed.
    #[error("autostart could not be changed")]
    Autostart,
    /// A file or folder could not be opened.
    #[error("could not open the file")]
    Open,
    /// The paste keystroke could not be sent.
    #[error("the paste keystroke could not be sent")]
    Paste,
    /// The recording overlay window could not be created.
    #[error("the recording overlay could not be shown")]
    Overlay,
    /// A file's permissions could not be restricted.
    #[error("file permissions could not be restricted")]
    Permissions,
}

impl PlatformError {
    /// Stable error code safe to show to users.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::HotkeyTaken(_) => "hotkey_in_use",
            Self::Hotkey => "hotkey_failed",
            Self::Clipboard => "clipboard_failed",
            Self::Paste => "paste_failed",
            Self::Open => "open_failed",
            Self::Control => "control_failed",
            Self::AlreadyRunning => "already_running",
            Self::NotRunning => "not_running",
            Self::ForeignPipe => "foreign_pipe",
            Self::Autostart => "autostart_failed",
            Self::Overlay => "overlay_failed",
            Self::Permissions => "permissions_failed",
        }
    }
}

/// What the tray icon shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayState {
    /// Ready.
    Idle = 0,
    /// The microphone is recording.
    Recording = 1,
    /// A dictation is being transcribed.
    Processing = 2,
    /// The last dictation may be misheard.
    Warning = 3,
    /// Something failed (details in the notification).
    Error = 4,
}

/// A tray menu entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    /// Start or stop a recording.
    Toggle,
    /// Paste the last dictation again.
    Repaste,
    /// Paste the last raw transcript.
    RepasteRaw,
    /// Open the settings.
    Settings,
    /// Open the log folder.
    Logs,
    /// Quit the daemon.
    Quit,
    /// Open the page of the newer release (shown once one is offered).
    Update,
}

/// Input from the desktop thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesktopEvent {
    /// A shortcut went down or up.
    Hotkey(HotkeyEvent),
    /// A tray menu entry was chosen.
    Menu(MenuAction),
}
