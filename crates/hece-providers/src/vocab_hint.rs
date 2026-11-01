//! Renders the vocabulary in the form a provider accepts ([`VocabularyStyle`]).
//!
//! Remote providers do not expose their tokenizer, so token counts are a conservative estimate
//! (see [`estimate_tokens`]); the local engine counts with the real Whisper tokenizer instead.

use hece_core::Vocabulary;

use crate::provider::VocabularyStyle;

/// The vocabulary as a provider takes it.
#[derive(Clone, PartialEq, Eq)]
pub enum VocabularyHint {
    /// Nothing to send.
    None,
    /// A Whisper-style text prompt.
    Prompt(String),
    /// Separate key terms, highest priority first.
    Keyterms(Vec<String>),
}

impl std::fmt::Debug for VocabularyHint {
    /// Redacted: terms are personal data, so only sizes show.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => f.write_str("None"),
            Self::Prompt(p) => write!(f, "Prompt({} chars)", p.chars().count()),
            Self::Keyterms(t) => write!(f, "Keyterms({} terms)", t.len()),
        }
    }
}

/// Upper-bound token estimate for BPE tokenizers: one token per two UTF-8 bytes. Real counts
/// are lower for ASCII words and close to this for Turkish letters outside ASCII.
#[must_use]
pub fn estimate_tokens(text: &str) -> usize {
    text.len().div_ceil(2)
}

/// Renders `vocab` for `style`. `prompt_tokens` is the user's prompt budget; the style's own
/// limit caps it.
#[must_use]
pub fn render(
    style: VocabularyStyle,
    vocab: Option<&Vocabulary>,
    prefix: &str,
    prompt_tokens: usize,
) -> VocabularyHint {
    let Some(vocab) = vocab.filter(|v| !v.terms.is_empty()) else {
        return VocabularyHint::None;
    };
    match style {
        VocabularyStyle::Unsupported => VocabularyHint::None,
        VocabularyStyle::WhisperPrompt { max_tokens } => vocab
            .whisper_prompt(prefix, &estimate_tokens, prompt_tokens.min(max_tokens))
            .map_or(VocabularyHint::None, VocabularyHint::Prompt),
        VocabularyStyle::Keyterms {
            max_terms,
            max_tokens,
        } => {
            let mut used = 0;
            let terms: Vec<String> = vocab
                .terms
                .iter()
                .take(max_terms)
                .take_while(|t| {
                    used += estimate_tokens(t);
                    used <= max_tokens
                })
                .cloned()
                .collect();
            if terms.is_empty() {
                VocabularyHint::None
            } else {
                VocabularyHint::Keyterms(terms)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vocab(terms: &[&str]) -> Vocabulary {
        Vocabulary {
            terms: terms.iter().map(|s| (*s).to_owned()).collect(),
        }
    }

    #[test]
    fn empty_or_missing_vocabulary_renders_nothing() {
        let style = VocabularyStyle::WhisperPrompt { max_tokens: 224 };
        assert_eq!(render(style, None, "V: ", 200), VocabularyHint::None);
        assert_eq!(
            render(style, Some(&vocab(&[])), "V: ", 200),
            VocabularyHint::None
        );
    }

    #[test]
    fn unsupported_style_renders_nothing() {
        let v = vocab(&["Kubernetes"]);
        assert_eq!(
            render(VocabularyStyle::Unsupported, Some(&v), "V: ", 200),
            VocabularyHint::None
        );
    }

    #[test]
    fn prompt_respects_the_smaller_budget() {
        let v = vocab(&["Kubernetes", "PostgreSQL", "Tauri", "İstanbul"]);
        let style = VocabularyStyle::WhisperPrompt { max_tokens: 224 };
        assert_eq!(
            render(style, Some(&v), "Vocabulary: ", 200),
            VocabularyHint::Prompt("Vocabulary: Kubernetes, PostgreSQL, Tauri, İstanbul.".into())
        );
        // " Vocabulary: Kubernetes." is 24 bytes = 12 estimated tokens.
        assert_eq!(
            render(style, Some(&v), "Vocabulary: ", 12),
            VocabularyHint::Prompt("Vocabulary: Kubernetes.".into())
        );
        assert_eq!(
            render(style, Some(&v), "Vocabulary: ", 5),
            VocabularyHint::None
        );
    }

    #[test]
    fn keyterms_stop_at_term_and_token_limits() {
        let v = vocab(&["aaaa", "bbbb", "cccc", "dddd"]);
        let by_terms = VocabularyStyle::Keyterms {
            max_terms: 2,
            max_tokens: 500,
        };
        assert_eq!(
            render(by_terms, Some(&v), "", 0),
            VocabularyHint::Keyterms(vec!["aaaa".into(), "bbbb".into()])
        );
        let by_tokens = VocabularyStyle::Keyterms {
            max_terms: 50,
            max_tokens: 3,
        };
        assert_eq!(
            render(by_tokens, Some(&v), "", 0),
            VocabularyHint::Keyterms(vec!["aaaa".into()])
        );
    }

    #[test]
    fn debug_hides_terms() {
        let shown = format!("{:?}", VocabularyHint::Keyterms(vec!["Gizli".into()]));
        assert!(!shown.contains("Gizli"));
    }
}
