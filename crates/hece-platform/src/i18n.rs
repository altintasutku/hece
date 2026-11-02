//! The language of user-facing texts: English, or Turkish when the system UI language is
//! Turkish. The settings window makes the same choice from `navigator.language`. Error codes
//! and logs stay language-neutral.

use std::sync::OnceLock;

use crate::{MenuAction, TrayState};

/// A language Hece's texts are written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    /// English, the default.
    En,
    /// Turkish.
    Tr,
}

/// The UI language, read once per process.
#[must_use]
pub fn ui_language() -> Lang {
    static LANG: OnceLock<Lang> = OnceLock::new();
    *LANG.get_or_init(detect)
}

#[cfg(windows)]
fn detect() -> Lang {
    from_langid(crate::windows::user_ui_language())
}

#[cfg(not(windows))]
fn detect() -> Lang {
    Lang::En
}

/// `LANG_TURKISH` is primary language 0x1f; the sublanguage sits above the low 10 bits.
#[cfg_attr(
    not(windows),
    allow(dead_code, reason = "only Windows reports a LANGID")
)]
fn from_langid(langid: u16) -> Lang {
    if langid & 0x3ff == 0x1f {
        Lang::Tr
    } else {
        Lang::En
    }
}

/// A tray menu entry's label.
#[must_use]
pub fn menu_label(action: MenuAction, lang: Lang) -> &'static str {
    match (lang, action) {
        (Lang::En, MenuAction::Toggle) => "Start / stop recording",
        (Lang::En, MenuAction::Repaste) => "Paste last dictation",
        (Lang::En, MenuAction::RepasteRaw) => "Paste last raw transcript",
        (Lang::En, MenuAction::Settings) => "Settings…",
        (Lang::En, MenuAction::Logs) => "Open log folder",
        (Lang::En, MenuAction::Quit) => "Quit Hece",
        (Lang::En, MenuAction::Update) => "Update available",
        (Lang::Tr, MenuAction::Toggle) => "Kaydı başlat / bitir",
        (Lang::Tr, MenuAction::Repaste) => "Son dikteyi yapıştır",
        (Lang::Tr, MenuAction::RepasteRaw) => "Son ham metni yapıştır",
        (Lang::Tr, MenuAction::Settings) => "Ayarlar…",
        (Lang::Tr, MenuAction::Logs) => "Kayıt klasörünü aç",
        (Lang::Tr, MenuAction::Quit) => "Hece'den çık",
        (Lang::Tr, MenuAction::Update) => "Yeni sürüm var",
    }
}

/// What the tray tooltip says after "Hece: ". `shortcut` is the record shortcut.
#[must_use]
pub fn tray_status(state: TrayState, lang: Lang, shortcut: &str) -> String {
    match (lang, state) {
        (Lang::En, TrayState::Idle) => format!("ready ({shortcut})"),
        (Lang::En, TrayState::Recording) => "recording".to_owned(),
        (Lang::En, TrayState::Processing) => "transcribing".to_owned(),
        (Lang::En, TrayState::Warning) => "may be misheard".to_owned(),
        (Lang::En, TrayState::Error) => "something failed; see the notification".to_owned(),
        (Lang::Tr, TrayState::Idle) => format!("hazır ({shortcut})"),
        (Lang::Tr, TrayState::Recording) => "kaydediyor".to_owned(),
        (Lang::Tr, TrayState::Processing) => "yazıya çeviriyor".to_owned(),
        (Lang::Tr, TrayState::Warning) => "yanlış duyulmuş olabilir".to_owned(),
        (Lang::Tr, TrayState::Error) => "bir şey başarısız oldu; bildirime bak".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turkish_langid_ignores_the_sublanguage() {
        assert_eq!(from_langid(0x041f), Lang::Tr);
        assert_eq!(from_langid(0x0409), Lang::En);
        assert_eq!(from_langid(0x0407), Lang::En);
    }
}
