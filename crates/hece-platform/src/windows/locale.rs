//! The user's Windows display language.
#![allow(unsafe_code, reason = "Win32 locale FFI wrapper")]

use windows_sys::Win32::Globalization::GetUserDefaultUILanguage;

/// The LANGID of the user's display language (what Settings > Time & language shows).
pub fn user_ui_language() -> u16 {
    // SAFETY: GetUserDefaultUILanguage takes no arguments and has no preconditions.
    unsafe { GetUserDefaultUILanguage() }
}
