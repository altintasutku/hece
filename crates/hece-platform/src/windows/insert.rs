//! Insertion: clipboard + one Ctrl+V into the window that had focus when recording stopped.
//! Transcripts are kept out of Win+V history and the cloud clipboard, and the user's previous
//! clipboard text is put back afterwards.
#![allow(unsafe_code, reason = "Win32 input and window FFI wrapper")]

use std::time::{Duration, Instant};

use arboard::{Clipboard, SetExtWindows};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput,
    VIRTUAL_KEY, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT, VK_V,
};
use windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

use crate::PlatformError;

/// Longest wait for the user to let go of the shortcut's modifiers before pasting; pasting
/// while Ctrl+Shift is still down would send Ctrl+Shift+V.
const MODIFIER_WAIT: Duration = Duration::from_secs(2);
/// Time the target gets to read the clipboard before the previous text is put back.
const PASTE_SETTLE: Duration = Duration::from_millis(300);

/// A top-level window, captured when recording stops.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window(isize);

/// The window that has keyboard focus, if any.
#[must_use]
pub fn focused_window() -> Option<Window> {
    // SAFETY: GetForegroundWindow has no preconditions.
    let hwnd = unsafe { GetForegroundWindow() };
    (!hwnd.is_null()).then_some(Window(hwnd as isize))
}

/// How an insert ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Inserted {
    /// Pasted into the target window.
    Pasted,
    /// Focus moved to another window, so nothing was pasted; the text stays on the clipboard.
    FocusChanged,
}

fn set_private(clipboard: &mut Clipboard, text: &str) -> Result<(), PlatformError> {
    clipboard
        .set()
        .exclude_from_monitoring()
        .exclude_from_history()
        .exclude_from_cloud()
        .text(text)
        .map_err(|_| PlatformError::Clipboard)
}

fn key_down(vk: VIRTUAL_KEY) -> bool {
    // SAFETY: GetAsyncKeyState takes any virtual-key code.
    unsafe { GetAsyncKeyState(i32::from(vk)) < 0 }
}

fn wait_for_modifiers_released() {
    let deadline = Instant::now() + MODIFIER_WAIT;
    while [VK_CONTROL, VK_MENU, VK_SHIFT, VK_LWIN, VK_RWIN]
        .into_iter()
        .any(key_down)
        && Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn key(vk: VIRTUAL_KEY, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: if up { KEYEVENTF_KEYUP } else { 0 },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn send_paste() -> Result<(), PlatformError> {
    let inputs = [
        key(VK_CONTROL, false),
        key(VK_V, false),
        key(VK_V, true),
        key(VK_CONTROL, true),
    ];
    let size = i32::try_from(std::mem::size_of::<INPUT>()).map_err(|_| PlatformError::Paste)?;
    // SAFETY: `inputs` is a valid array of INPUT structs whose length is passed alongside it,
    // and `size` is the size of one element.
    let sent = unsafe { SendInput(4, inputs.as_ptr(), size) };
    if sent == 4 {
        Ok(())
    } else {
        Err(PlatformError::Paste)
    }
}

/// Puts `text` on the clipboard (kept out of history and the cloud clipboard) without pasting.
pub fn copy(text: &str) -> Result<(), PlatformError> {
    let mut clipboard = Clipboard::new().map_err(|_| PlatformError::Clipboard)?;
    set_private(&mut clipboard, text)
}

/// Pastes `text` into `target` (or wherever focus is when `target` is `None`). Blocks for up
/// to a few seconds; call it from a blocking context.
///
/// Windows does not report a paste blocked by UIPI (an elevated target window), so that case
/// looks like [`Inserted::Pasted`].
pub fn insert(text: &str, target: Option<Window>) -> Result<Inserted, PlatformError> {
    let mut clipboard = Clipboard::new().map_err(|_| PlatformError::Clipboard)?;
    let previous = clipboard.get_text().ok();
    set_private(&mut clipboard, text)?;
    wait_for_modifiers_released();
    if target.is_some() && focused_window() != target {
        return Ok(Inserted::FocusChanged);
    }
    send_paste()?;
    std::thread::sleep(PASTE_SETTLE);
    if let Some(previous) = previous
        && set_private(&mut clipboard, &previous).is_err()
    {
        tracing::debug!("previous clipboard text could not be restored");
    }
    Ok(Inserted::Pasted)
}
