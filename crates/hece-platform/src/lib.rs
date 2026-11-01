//! Desktop integration. Windows: global hotkeys (`RegisterHotKey`), insertion as clipboard + one
//! Ctrl+V, and the focused window. Other platforms have no implementation yet.

#[cfg(windows)]
mod windows;

#[cfg(windows)]
pub use windows::{
    CANCEL_SHORTCUT, Hotkeys, Inserted, RECORD_SHORTCUT, Window, focused_window, insert,
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
    HotkeyTaken(&'static str),
    /// Setting up hotkeys failed.
    #[error("global shortcuts could not be set up")]
    Hotkey,
    /// The clipboard could not be used.
    #[error("the clipboard is not available")]
    Clipboard,
    /// The paste keystroke could not be sent.
    #[error("the paste keystroke could not be sent")]
    Paste,
}

impl PlatformError {
    /// Stable error code safe to show to users.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::HotkeyTaken(_) => "hotkey_taken",
            Self::Hotkey => "hotkey_failed",
            Self::Clipboard => "clipboard_failed",
            Self::Paste => "paste_failed",
        }
    }
}
