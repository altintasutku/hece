//! Guard for the LLM cleanup step.
//!
//! The cleanup step may only make minimal edits (fillers, punctuation, repeats). The
//! guard rejects outputs that dropped a negation, came back empty, or changed length
//! sharply, so the caller can insert the raw text instead. Reasons carry no text, so
//! they are safe to log.

use std::sync::LazyLock;

use regex::Regex;

use crate::text::fold;

/// Why a cleaned text was rejected. Carries no text, so it is safe to log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectReason {
    /// The cleaned text has no letters or digits while the raw text does.
    Empty,
    /// The cleaned text has fewer negation tokens than the raw text.
    NegationDropped,
    /// The cleaned text is sharply shorter than the raw text.
    TooShort,
    /// The cleaned text is sharply longer than the raw text.
    TooLong,
}

impl RejectReason {
    /// Stable machine-readable code for logs and error values.
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::Empty => "cleanup_empty",
            Self::NegationDropped => "cleanup_negation_dropped",
            Self::TooShort => "cleanup_too_short",
            Self::TooLong => "cleanup_too_long",
        }
    }
}

/// Outcome of [`cleanup_guard`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardVerdict {
    /// The cleaned text may replace the raw transcript.
    Accept,
    /// The cleaned text must be discarded and the raw text inserted instead.
    Reject(RejectReason),
}

/// Cleaned text shorter than this fraction of the raw letter count is suspect.
const MIN_LENGTH_RATIO: f64 = 0.6;
/// Cleaned text longer than this multiple of the raw letter count is suspect.
const MAX_LENGTH_RATIO: f64 = 1.2;
/// Letters of absolute slack so short texts are not rejected over a few characters.
const LENGTH_SLACK: f64 = 8.0;

#[expect(
    clippy::expect_used,
    reason = "static regex built from a literal; invalid literal is a programmer error"
)]
static TOKEN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[\p{L}\p{N}'’]+").expect("valid regex literal"));

fn is_negation(token: &str) -> bool {
    token.starts_with("degil")
        || token.starts_with("yok")
        || token.ends_with("n't")
        || token.ends_with("n’t")
        || token.starts_with("hic")
        || matches!(
            token,
            "hayir"
                | "asla"
                | "not"
                | "no"
                | "never"
                | "cannot"
                | "nothing"
                | "nobody"
                | "none"
                | "neither"
                | "nor"
                | "without"
        )
}

fn negation_count(text: &str) -> usize {
    let folded = fold(text);
    TOKEN
        .find_iter(&folded)
        .filter(|m| is_negation(m.as_str()))
        .count()
}

fn letter_count(text: &str) -> usize {
    text.chars().filter(|c| c.is_alphanumeric()).count()
}

/// Decides whether the LLM-cleaned text may replace the raw transcript.
///
/// Checks, in order: empty output, dropped negations, too short, too long. Only
/// Turkish word-level negation (`değil…`, `yok…`, `hiç…`, `hayır`, `asla`) and English
/// negations (`not`, `no`, `never`, `cannot`, `n't`, `nothing`, `nobody`, `none`,
/// `neither`, `nor`, `without`) are detected; Turkish verb negation (`-me/-ma`) is out
/// of scope. The `yok` and `hic` prefixes also match some non-negations (e.g. `yoksa`,
/// `hiciv`); this errs on the safe side, since a rejection just inserts the raw text.
#[must_use]
pub fn cleanup_guard(raw: &str, cleaned: &str) -> GuardVerdict {
    let raw_letters = letter_count(raw);
    let cleaned_letters = letter_count(cleaned);

    if raw_letters == 0 && cleaned_letters == 0 {
        return GuardVerdict::Accept;
    }
    if cleaned_letters == 0 {
        return GuardVerdict::Reject(RejectReason::Empty);
    }
    if negation_count(cleaned) < negation_count(raw) {
        return GuardVerdict::Reject(RejectReason::NegationDropped);
    }

    // Letter counts are small; the usize -> f64 conversion is exact.
    #[expect(
        clippy::cast_precision_loss,
        reason = "text lengths are far below 2^52"
    )]
    let (raw_len, cleaned_len) = (raw_letters as f64, cleaned_letters as f64);
    if cleaned_len < raw_len * MIN_LENGTH_RATIO && raw_len - cleaned_len > LENGTH_SLACK {
        return GuardVerdict::Reject(RejectReason::TooShort);
    }
    if cleaned_len > raw_len * MAX_LENGTH_RATIO + LENGTH_SLACK {
        return GuardVerdict::Reject(RejectReason::TooLong);
    }
    GuardVerdict::Accept
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_minimal_cleanup() {
        assert_eq!(
            cleanup_guard("ııı şey bence bu doğru değil yani", "Bence bu doğru değil."),
            GuardVerdict::Accept
        );
        assert_eq!(cleanup_guard("ııı tamam", "Tamam."), GuardVerdict::Accept);
        assert_eq!(
            cleanup_guard("merhaba dünya", "Merhaba, dünya!"),
            GuardVerdict::Accept
        );
    }

    #[test]
    fn rejects_dropped_negations() {
        let cases = [
            ("bu doğru değil", "bu doğru"),
            ("dosya yoktu", "dosya vardı"),
            ("hayır gitme", "gitme"),
            ("I do not agree", "I agree"),
            ("I don't agree", "I agree"),
            ("never push to main", "push to main"),
            ("değil değil", "değil"),
            ("ben değilim", "ben"),
            ("I don\u{2019}t agree", "I agree"),
            ("bunu asla yapma", "bunu yapma"),
            ("hiçbir şey olmadı", "bir şey olmadı"),
            ("nothing changed here", "changed here"),
        ];
        for (raw, cleaned) in cases {
            assert_eq!(
                cleanup_guard(raw, cleaned),
                GuardVerdict::Reject(RejectReason::NegationDropped),
                "{raw:?} -> {cleaned:?}"
            );
        }
    }

    #[test]
    fn accepts_when_negation_count_rises() {
        assert_eq!(cleanup_guard("değil", "değil değil"), GuardVerdict::Accept);
    }

    #[test]
    fn empty_cases() {
        assert_eq!(
            cleanup_guard("some text", "  "),
            GuardVerdict::Reject(RejectReason::Empty)
        );
        assert_eq!(cleanup_guard("", ""), GuardVerdict::Accept);
    }

    #[test]
    fn rejects_sharp_length_changes() {
        // "abcd " x25 = 100 letters; x10 = 40 letters.
        assert_eq!(
            cleanup_guard(&"abcd ".repeat(25), &"abcd ".repeat(10)),
            GuardVerdict::Reject(RejectReason::TooShort)
        );
        // "abcd " x5 = 20 letters; x50 = 200 letters.
        assert_eq!(
            cleanup_guard(&"abcd ".repeat(5), &"abcd ".repeat(50)),
            GuardVerdict::Reject(RejectReason::TooLong)
        );
    }

    #[test]
    fn reason_codes() {
        assert_eq!(RejectReason::Empty.code(), "cleanup_empty");
        assert_eq!(
            RejectReason::NegationDropped.code(),
            "cleanup_negation_dropped"
        );
        assert_eq!(RejectReason::TooShort.code(), "cleanup_too_short");
        assert_eq!(RejectReason::TooLong.code(), "cleanup_too_long");
    }
}
