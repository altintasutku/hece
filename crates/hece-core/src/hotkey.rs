//! Global shortcut strings such as `Ctrl+Shift+Space`: parsed, validated and printed in one
//! canonical form (`Ctrl`, `Alt`, `Shift`, `Win`, then the key). The platform layer turns a
//! [`Hotkey`] into `RegisterHotKey` arguments through [`Key::code`], the W3C key code.

use std::fmt;

/// The default record shortcut.
pub const DEFAULT_RECORD: &str = "Ctrl+Shift+Space";
/// The default cancel shortcut.
pub const DEFAULT_CANCEL: &str = "Ctrl+Shift+Backspace";

/// Named keys: config name and W3C code. Punctuation is left out on purpose: its virtual key
/// depends on the keyboard layout (a Turkish Q layout moves most of it).
const NAMED: &[(&str, &str)] = &[
    ("Space", "Space"),
    ("Enter", "Enter"),
    ("Tab", "Tab"),
    ("Backspace", "Backspace"),
    ("Delete", "Delete"),
    ("Insert", "Insert"),
    ("Home", "Home"),
    ("End", "End"),
    ("PageUp", "PageUp"),
    ("PageDown", "PageDown"),
    ("Up", "ArrowUp"),
    ("Down", "ArrowDown"),
    ("Left", "ArrowLeft"),
    ("Right", "ArrowRight"),
    ("Pause", "Pause"),
];

/// Combos Windows keeps for itself, or that would break Hece's own paste chord.
const RESERVED: &[&str] = &[
    "Ctrl+Alt+Delete",
    "Alt+Tab",
    "Alt+F4",
    "Win+L",
    "Ctrl+V",
    "Shift+Insert",
];

/// Modifier keys held with the main key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "four independent modifier flags, as Windows has them"
)]
pub struct Modifiers {
    /// Ctrl.
    pub ctrl: bool,
    /// Alt.
    pub alt: bool,
    /// Shift.
    pub shift: bool,
    /// The Windows key.
    pub win: bool,
}

/// The main key of a shortcut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    /// An entry of the named-key table.
    Named(&'static str, &'static str),
    /// `A`-`Z`.
    Letter(char),
    /// `0`-`9` on the main row.
    Digit(char),
    /// `F1`-`F24`.
    Function(u8),
}

impl Key {
    fn parse(token: &str) -> Option<Self> {
        if let Some(&(name, code)) = NAMED.iter().find(|(name, code)| {
            token.eq_ignore_ascii_case(name) || token.eq_ignore_ascii_case(code)
        }) {
            return Some(Self::Named(name, code));
        }
        let upper = token.to_ascii_uppercase();
        let single = upper
            .strip_prefix("KEY")
            .or_else(|| upper.strip_prefix("DIGIT"))
            .unwrap_or(&upper);
        let mut chars = single.chars();
        if let (Some(c), None) = (chars.next(), chars.next()) {
            if c.is_ascii_uppercase() {
                return Some(Self::Letter(c));
            }
            if c.is_ascii_digit() {
                return Some(Self::Digit(c));
            }
        }
        let n: u8 = upper.strip_prefix('F')?.parse().ok()?;
        (1..=24).contains(&n).then_some(Self::Function(n))
    }

    /// The W3C `KeyboardEvent.code` of the key (`Space`, `KeyA`, `Digit1`, `F13`, `ArrowUp`).
    #[must_use]
    pub fn code(&self) -> String {
        match self {
            Self::Named(_, code) => (*code).to_owned(),
            Self::Letter(c) => format!("Key{c}"),
            Self::Digit(c) => format!("Digit{c}"),
            Self::Function(n) => format!("F{n}"),
        }
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Named(name, _) => f.write_str(name),
            Self::Letter(c) | Self::Digit(c) => write!(f, "{c}"),
            Self::Function(n) => write!(f, "F{n}"),
        }
    }
}

/// A validated global shortcut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hotkey {
    /// Keys held with [`Hotkey::key`].
    pub modifiers: Modifiers,
    /// The key that fires the shortcut.
    pub key: Key,
}

/// Why a shortcut string was refused. [`HotkeyError::code`] is the stable user-facing code.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HotkeyError {
    /// Not `Modifier+...+Key` with one known key.
    #[error("is not a shortcut like Ctrl+Shift+Space")]
    Invalid,
    /// No Ctrl, Alt or Win, so it would fire while typing.
    #[error("needs Ctrl, Alt or Win (only F13-F24 work alone)")]
    NoModifier,
    /// Windows keeps it, or it is Hece's paste chord.
    #[error("is reserved by Windows or used by Hece to paste")]
    Reserved,
}

impl HotkeyError {
    /// Stable error code safe to show to users.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::Invalid => "hotkey_invalid",
            Self::NoModifier => "hotkey_no_modifier",
            Self::Reserved => "hotkey_reserved",
        }
    }
}

impl std::str::FromStr for Hotkey {
    type Err = HotkeyError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let mut modifiers = Modifiers::default();
        let mut key = None;
        for token in text.split('+').map(str::trim) {
            if key.is_some() {
                // The key comes last, once.
                return Err(HotkeyError::Invalid);
            }
            let flag = match token.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => &mut modifiers.ctrl,
                "alt" => &mut modifiers.alt,
                "shift" => &mut modifiers.shift,
                "win" | "super" | "meta" => &mut modifiers.win,
                _ => {
                    key = Some(Key::parse(token).ok_or(HotkeyError::Invalid)?);
                    continue;
                }
            };
            if *flag {
                return Err(HotkeyError::Invalid);
            }
            *flag = true;
        }
        let hotkey = Self {
            modifiers,
            key: key.ok_or(HotkeyError::Invalid)?,
        };
        let Modifiers { ctrl, alt, win, .. } = modifiers;
        let function = matches!(hotkey.key, Key::Function(_));
        let alone_ok = matches!(hotkey.key, Key::Function(13..)) && !modifiers.shift;
        // Shift alone still types (Shift+A); it only counts next to a function key.
        if !(ctrl || alt || win || (modifiers.shift && function) || alone_ok) {
            return Err(HotkeyError::NoModifier);
        }
        if RESERVED.contains(&hotkey.to_string().as_str()) {
            return Err(HotkeyError::Reserved);
        }
        Ok(hotkey)
    }
}

impl fmt::Display for Hotkey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let m = self.modifiers;
        for (on, name) in [
            (m.ctrl, "Ctrl"),
            (m.alt, "Alt"),
            (m.shift, "Shift"),
            (m.win, "Win"),
        ] {
            if on {
                write!(f, "{name}+")?;
            }
        }
        write!(f, "{}", self.key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Result<String, &'static str> {
        text.parse::<Hotkey>()
            .map(|h| h.to_string())
            .map_err(|e| e.code())
    }

    #[test]
    fn parses_into_canonical_form() {
        assert_eq!(parse(DEFAULT_RECORD), Ok(DEFAULT_RECORD.to_owned()));
        assert_eq!(parse(DEFAULT_CANCEL), Ok(DEFAULT_CANCEL.to_owned()));
        assert_eq!(
            parse("shift + control + keyk"),
            Ok("Ctrl+Shift+K".to_owned())
        );
        assert_eq!(parse("Win+Alt+Digit5"), Ok("Alt+Win+5".to_owned()));
        assert_eq!(parse("ctrl+arrowup"), Ok("Ctrl+Up".to_owned()));
        assert_eq!(parse("F13"), Ok("F13".to_owned()));
        assert_eq!(parse("Shift+F9"), Ok("Shift+F9".to_owned()));
        let h: Hotkey = "Ctrl+Shift+Space".parse().unwrap();
        assert_eq!(h.key.code(), "Space");
    }

    #[test]
    fn refuses_bad_shortcuts() {
        for bad in [
            "",
            "Ctrl+",
            "Ctrl+Shift",
            "Ctrl+A+B",
            "A+Ctrl",
            "Ctrl+Ctrl+A",
            "Ctrl+Ö",
            "Ctrl+F25",
        ] {
            assert_eq!(parse(bad), Err("hotkey_invalid"), "{bad}");
        }
        for typing in ["Space", "A", "Shift+A", "F5", "Shift+Space"] {
            assert_eq!(parse(typing), Err("hotkey_no_modifier"), "{typing}");
        }
        for reserved in ["Alt+F4", "ctrl+v", "Ctrl+Alt+Delete"] {
            assert_eq!(parse(reserved), Err("hotkey_reserved"), "{reserved}");
        }
    }
}
