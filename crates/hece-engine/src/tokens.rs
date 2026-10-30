//! Whisper special tokens, the suppress list and prompt helpers.
//!
//! Ported from faster-whisper 1.2.1 (MIT), `tokenizer.py` (`Tokenizer`, `non_speech_tokens`) and
//! `transcribe.py` (`get_suppressed_tokens`).

use std::collections::BTreeSet;

use hece_core::Vocabulary;

use crate::error::EngineError;

/// The text-tokenizer operations the pipeline needs, so tests can run without a real model.
pub trait TextTokenizer {
    /// Encodes text without adding special tokens. Returns no ids if the tokenizer fails, which
    /// cannot happen for a successfully loaded tokenizer and valid UTF-8 input.
    fn encode(&self, text: &str) -> Vec<u32>;
    /// Decodes text tokens, skipping special tokens. Callers must pass text tokens only
    /// (ids below `eot`); timestamp tokens are not rendered.
    fn decode(&self, ids: &[u32]) -> String;
    /// Looks up the id of a token string such as `<|endoftext|>`.
    fn token_id(&self, token: &str) -> Option<u32>;
}

impl TextTokenizer for tokenizers::Tokenizer {
    fn encode(&self, text: &str) -> Vec<u32> {
        // `add_special_tokens = false` matches faster-whisper's `Tokenizer.encode`.
        // `Tokenizer` derefs to the type holding the inherent methods; deref explicitly so the
        // call does not resolve back to this trait method.
        (**self)
            .encode(text, false)
            .map(|e| e.get_ids().to_vec())
            .unwrap_or_default()
    }

    fn decode(&self, ids: &[u32]) -> String {
        (**self).decode(ids, true).unwrap_or_default()
    }

    fn token_id(&self, token: &str) -> Option<u32> {
        self.token_to_id(token)
    }
}

/// Ids of the special tokens the decoder uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpecialTokens {
    /// `<|startoftranscript|>`.
    pub sot: u32,
    /// `<|startofprev|>`, which introduces the previous-text prompt.
    pub sot_prev: u32,
    /// `<|startoflm|>`.
    pub sot_lm: u32,
    /// `<|endoftext|>`.
    pub eot: u32,
    /// `<|transcribe|>`.
    pub transcribe: u32,
    /// `<|translate|>`.
    pub translate: u32,
    /// `<|notimestamps|>`.
    pub no_timestamps: u32,
    /// `<|nospeech|>`, or `<|nocaptions|>` on older vocabularies.
    pub no_speech: u32,
    /// First timestamp token (`no_timestamps + 1`).
    pub timestamp_begin: u32,
}

fn require(tok: &dyn TextTokenizer, token: &str) -> Result<u32, EngineError> {
    tok.token_id(token)
        .ok_or_else(|| EngineError::Tokenizer(format!("missing token {token}")))
}

impl SpecialTokens {
    /// Looks up every special token.
    ///
    /// # Errors
    /// `EngineError::Tokenizer` if the vocabulary lacks one of them.
    pub fn resolve(tok: &dyn TextTokenizer) -> Result<Self, EngineError> {
        let no_timestamps = require(tok, "<|notimestamps|>")?;
        let no_speech = match tok.token_id("<|nospeech|>") {
            Some(id) => id,
            None => require(tok, "<|nocaptions|>")?,
        };
        Ok(Self {
            sot: require(tok, "<|startoftranscript|>")?,
            sot_prev: require(tok, "<|startofprev|>")?,
            sot_lm: require(tok, "<|startoflm|>")?,
            eot: require(tok, "<|endoftext|>")?,
            transcribe: require(tok, "<|transcribe|>")?,
            translate: require(tok, "<|translate|>")?,
            no_timestamps,
            no_speech,
            timestamp_begin: no_timestamps + 1,
        })
    }
}

/// The language token for a code such as `tr` (`<|tr|>`).
///
/// # Errors
/// `EngineError::Tokenizer` if the vocabulary has no such language.
pub fn language_token(tok: &dyn TextTokenizer, lang: &str) -> Result<u32, EngineError> {
    require(tok, &format!("<|{lang}|>"))
}

/// The decoder prompt `[sot, language, transcribe]`.
#[must_use]
pub fn sot_sequence(sp: &SpecialTokens, language: u32) -> [u32; 3] {
    [sp.sot, language, sp.transcribe]
}

const SYMBOLS: &str = "\"#()*+/:;<=>@[\\]^_`{|}~「」『』";
const SYMBOL_WORDS: &str = "<< >> <<< >>> -- --- -( -[ (' (\" (( )) ((( ))) [[ ]] {{ }} ♪♪ ♪♪♪";
const MISCELLANEOUS: &str = "♩♪♫♬♭♮♯";

/// Ids of speaker tags and non-speech annotations, sorted. Port of `Tokenizer.non_speech_tokens`.
fn non_speech_tokens(tok: &dyn TextTokenizer) -> BTreeSet<i32> {
    let mut result = BTreeSet::new();
    // Hyphens and single quotes are allowed inside words but not at the start of one.
    for s in [" -", " '"] {
        if let Some(&id) = tok.encode(s).first() {
            result.insert(id);
        }
    }
    let singles = SYMBOLS.chars().map(String::from);
    let words = SYMBOL_WORDS.split(' ').map(str::to_owned);
    let misc = MISCELLANEOUS.chars().map(String::from);
    let plain = singles.chain(words).map(|s| (s, false));
    for (symbol, is_misc) in plain.chain(misc.map(|s| (s, true))) {
        for ids in [tok.encode(&symbol), tok.encode(&format!(" {symbol}"))] {
            // Miscellaneous symbols may span several tokens; their first token is safe to drop.
            if (ids.len() == 1 || is_misc)
                && let Some(&id) = ids.first()
            {
                result.insert(id);
            }
        }
    }
    result
        .into_iter()
        .filter_map(|id| i32::try_from(id).ok())
        .collect()
}

/// The ids to suppress while decoding: `non_speech_tokens` plus the control tokens, sorted and
/// deduplicated. Port of `get_suppressed_tokens` with `suppress_tokens = [-1]`.
#[must_use]
pub fn suppressed_tokens(tok: &dyn TextTokenizer, sp: &SpecialTokens) -> Vec<i32> {
    let mut set = non_speech_tokens(tok);
    for id in [
        sp.transcribe,
        sp.translate,
        sp.sot,
        sp.sot_prev,
        sp.sot_lm,
        sp.no_speech,
    ] {
        if let Ok(id) = i32::try_from(id) {
            set.insert(id);
        }
    }
    set.into_iter().collect()
}

/// Tokens of the user prompt, with the leading space faster-whisper adds.
#[must_use]
pub fn initial_prompt_tokens(tok: &dyn TextTokenizer, prompt: &str) -> Vec<u32> {
    tok.encode(&format!(" {}", prompt.trim()))
}

/// Renders the vocabulary as a prompt that fits `max_tokens` (counted as the decoder sees it).
#[must_use]
pub fn vocabulary_prompt(
    tok: &dyn TextTokenizer,
    vocab: &Vocabulary,
    prefix: &str,
    max_tokens: usize,
) -> Option<String> {
    vocab.whisper_prompt(prefix, &|s| tok.encode(s).len(), max_tokens)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hece_core::Vocabulary;
    use std::collections::HashMap;

    /// Words split on spaces; every word is one token, `" "` prefix is its own token `0`.
    struct Fake {
        vocab: HashMap<String, u32>,
    }

    impl Fake {
        fn with(tokens: &[(&str, u32)]) -> Self {
            Self {
                vocab: tokens.iter().map(|(t, i)| ((*t).to_owned(), *i)).collect(),
            }
        }

        fn full() -> Self {
            Self::with(&[
                ("<|startoftranscript|>", 10),
                ("<|startofprev|>", 11),
                ("<|startoflm|>", 12),
                ("<|endoftext|>", 13),
                ("<|transcribe|>", 14),
                ("<|translate|>", 15),
                ("<|notimestamps|>", 16),
                ("<|nospeech|>", 17),
                ("<|tr|>", 18),
            ])
        }
    }

    impl TextTokenizer for Fake {
        fn encode(&self, text: &str) -> Vec<u32> {
            text.split(' ')
                .map(|w| u32::try_from(w.len()).unwrap() + 100)
                .collect()
        }
        fn decode(&self, _ids: &[u32]) -> String {
            String::new()
        }
        fn token_id(&self, token: &str) -> Option<u32> {
            self.vocab.get(token).copied()
        }
    }

    #[test]
    fn resolve_reads_all_special_tokens() {
        let sp = SpecialTokens::resolve(&Fake::full()).unwrap();
        assert_eq!(
            (
                sp.sot,
                sp.sot_prev,
                sp.sot_lm,
                sp.eot,
                sp.transcribe,
                sp.translate
            ),
            (10, 11, 12, 13, 14, 15)
        );
        assert_eq!(
            (sp.no_timestamps, sp.no_speech, sp.timestamp_begin),
            (16, 17, 17)
        );
    }

    #[test]
    fn resolve_falls_back_to_nocaptions() {
        let mut fake = Fake::full();
        fake.vocab.remove("<|nospeech|>");
        fake.vocab.insert("<|nocaptions|>".to_owned(), 99);
        assert_eq!(SpecialTokens::resolve(&fake).unwrap().no_speech, 99);
    }

    #[test]
    fn resolve_errors_on_missing_token() {
        let mut fake = Fake::full();
        fake.vocab.remove("<|endoftext|>");
        let err = SpecialTokens::resolve(&fake).unwrap_err();
        assert_eq!(err.code(), "tokenizer_failed");
    }

    #[test]
    fn language_token_and_sot_sequence() {
        let fake = Fake::full();
        let sp = SpecialTokens::resolve(&fake).unwrap();
        let lang = language_token(&fake, "tr").unwrap();
        assert_eq!(lang, 18);
        assert_eq!(sot_sequence(&sp, lang), [10, 18, 14]);
        assert!(language_token(&fake, "xx").is_err());
    }

    #[test]
    fn initial_prompt_trims_and_prepends_space() {
        let fake = Fake::full();
        assert_eq!(
            initial_prompt_tokens(&fake, "  ab cd \n"),
            fake.encode(" ab cd")
        );
    }

    #[test]
    fn vocabulary_prompt_respects_budget() {
        let fake = Fake::full();
        let vocab = Vocabulary {
            terms: vec!["aa".into(), "bb".into(), "cc".into()],
        };
        // " aa, bb, cc." splits into 3 words here; " aa." into 2.
        assert_eq!(
            vocabulary_prompt(&fake, &vocab, "", 2).as_deref(),
            Some("aa.")
        );
        assert_eq!(
            vocabulary_prompt(&fake, &vocab, "", 3).as_deref(),
            Some("aa, bb.")
        );
        assert_eq!(vocabulary_prompt(&fake, &vocab, "", 1), None);
    }

    #[test]
    fn suppressed_tokens_are_sorted_unique_and_include_specials() {
        let fake = Fake::full();
        let sp = SpecialTokens::resolve(&fake).unwrap();
        let list = suppressed_tokens(&fake, &sp);
        assert!(list.windows(2).all(|w| w[0] < w[1]));
        for id in [10, 11, 12, 14, 15, 17] {
            assert!(list.contains(&id));
        }
        assert!(!list.contains(&13));
    }
}
