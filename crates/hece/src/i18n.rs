//! Toast texts in the UI language ([`hece_platform::ui_language`]): English, or Turkish on a
//! Turkish Windows. Error details from `Display` impls and stable error codes are not
//! translated; CLI output and logs stay English.

use std::fmt;

use hece_platform::{Lang, ui_language};

/// A message the daemon shows as a toast.
pub(crate) enum Toast<'a> {
    ConfigOpenFailed,
    LogsOpenFailed,
    HistoryDisabled,
    HistoryEmpty,
    RepasteFailed,
    LimitReached,
    MicrophoneError(&'a str),
    FellBack {
        from: &'a str,
        reason: &'a str,
        to: &'a str,
    },
    NothingHeard,
    NotInserted(&'a dyn fmt::Display, &'a str),
    CleanupSkipped(&'a str),
    MayBeMisheard,
    FocusChanged,
    InsertFailed,
    ModelNotLoaded(&'a dyn fmt::Display, &'a str),
    ApiNotStarted(u16),
    NeedsSetupTitle,
    NeedsSetup,
    NotStartedTitle,
}

impl Toast<'_> {
    fn write(&self, lang: Lang, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match lang {
            Lang::En => self.english(f),
            Lang::Tr => self.turkish(f),
        }
    }

    fn english(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ConfigOpenFailed => f.write_str("Could not open the config file."),
            Self::LogsOpenFailed => f.write_str("Could not open the log folder."),
            Self::HistoryDisabled => {
                f.write_str("History is disabled, so there is nothing to paste again.")
            }
            Self::HistoryEmpty => f.write_str("History is empty."),
            Self::RepasteFailed => f.write_str("Could not paste again."),
            Self::LimitReached => {
                f.write_str("Recording limit reached; transcribing what was recorded.")
            }
            Self::MicrophoneError(code) => write!(f, "Microphone error ({code}); stopping."),
            Self::FellBack { from, reason, to } => {
                write!(
                    f,
                    "{from} failed ({reason}); {to} transcribed this dictation."
                )
            }
            Self::NothingHeard => f.write_str("Nothing heard."),
            Self::NotInserted(e, code) => write!(f, "Not inserted: {e} ({code})"),
            Self::CleanupSkipped(code) => {
                write!(f, "Cleanup skipped ({code}); inserted the raw transcript.")
            }
            Self::MayBeMisheard => f.write_str("Inserted, but it may be misheard."),
            Self::FocusChanged => f.write_str(
                "Focus changed, so nothing was pasted. The text is on the clipboard (Ctrl+V).",
            ),
            Self::InsertFailed => f.write_str("Insert failed."),
            Self::ModelNotLoaded(e, code) => {
                write!(f, "The local model could not be loaded: {e} ({code})")
            }
            Self::ApiNotStarted(port) => write!(
                f,
                "The local API did not start on port {port} (in use?); dictation still works."
            ),
            Self::NeedsSetupTitle => f.write_str("Hece needs setup"),
            Self::NeedsSetup => f.write_str(
                "Hece Settings is open: pick a model or a cloud provider, then press Start.",
            ),
            Self::NotStartedTitle => f.write_str("Hece could not start"),
        }
    }

    fn turkish(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ConfigOpenFailed => f.write_str("Ayar dosyası açılamadı."),
            Self::LogsOpenFailed => f.write_str("Kayıt klasörü açılamadı."),
            Self::HistoryDisabled => {
                f.write_str("Geçmiş kapalı, yeniden yapıştırılacak bir şey yok.")
            }
            Self::HistoryEmpty => f.write_str("Geçmiş boş."),
            Self::RepasteFailed => f.write_str("Yeniden yapıştırılamadı."),
            Self::LimitReached => {
                f.write_str("Kayıt sınırına ulaşıldı; kaydedilen kısım yazıya çevriliyor.")
            }
            Self::MicrophoneError(code) => write!(f, "Mikrofon hatası ({code}); kayıt durdu."),
            Self::FellBack { from, reason, to } => write!(
                f,
                "{from} başarısız oldu ({reason}); bu dikteyi {to} yazıya çevirdi."
            ),
            Self::NothingHeard => f.write_str("Hiçbir şey duyulmadı."),
            Self::NotInserted(e, code) => write!(f, "Eklenmedi: {e} ({code})"),
            Self::CleanupSkipped(code) => {
                write!(f, "Düzeltme atlandı ({code}); ham metin eklendi.")
            }
            Self::MayBeMisheard => f.write_str("Eklendi, ama yanlış duyulmuş olabilir."),
            Self::FocusChanged => {
                f.write_str("Odak değişti, bu yüzden yapıştırılmadı. Metin panoda (Ctrl+V).")
            }
            Self::InsertFailed => f.write_str("Eklenemedi."),
            Self::ModelNotLoaded(e, code) => write!(f, "Yerel model yüklenemedi: {e} ({code})"),
            Self::ApiNotStarted(port) => write!(
                f,
                "Yerel API {port} portunda başlamadı (kullanımda mı?); dikte yine çalışır."
            ),
            Self::NeedsSetupTitle => f.write_str("Hece'nin kurulması gerekiyor"),
            Self::NeedsSetup => f.write_str(
                "Hece Settings açık: bir model ya da bulut sağlayıcı seç, sonra Başlat'a bas.",
            ),
            Self::NotStartedTitle => f.write_str("Hece başlatılamadı"),
        }
    }
}

impl fmt::Display for Toast<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.write(ui_language(), f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct In<'a>(&'a Toast<'a>, Lang);

    impl fmt::Display for In<'_> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            self.0.write(self.1, f)
        }
    }

    #[test]
    fn codes_survive_translation() {
        let e = "detail";
        for toast in [
            Toast::MicrophoneError("code_x"),
            Toast::NotInserted(&e, "code_x"),
            Toast::CleanupSkipped("code_x"),
            Toast::ModelNotLoaded(&e, "code_x"),
        ] {
            for lang in [Lang::En, Lang::Tr] {
                let text = In(&toast, lang).to_string();
                assert!(text.contains("(code_x)"), "{text}");
            }
        }
    }
}
