//! Turkish-aware text folding used as the matching key across the core.

use std::sync::LazyLock;

use regex::Regex;
use unicode_normalization::{UnicodeNormalization, char::canonical_combining_class};

#[expect(
    clippy::expect_used,
    reason = "static regex built from a literal; invalid literal is a programmer error"
)]
static NON_WORD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[^\w]+|_+").expect("valid regex literal"));

/// Folds text for case- and diacritic-insensitive comparison.
///
/// NFKD, drop combining marks, lowercase, map dotless `ı` to `i`, trim. Rust's
/// `to_lowercase` differs from Python's `casefold` only for chars like `ß`; accepted.
#[must_use]
pub fn fold(text: &str) -> String {
    let stripped: String = text
        .nfkd()
        .filter(|c| canonical_combining_class(*c) == 0)
        .collect();
    stripped.to_lowercase().replace('ı', "i").trim().to_owned()
}

/// Builds a matching key: folds, turns punctuation and `_` runs into single spaces.
#[must_use]
pub fn normalize(text: &str) -> String {
    let folded = fold(text);
    let spaced = NON_WORD.replace_all(&folded, " ");
    spaced.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fold_unifies_dotted_and_dotless_i() {
        for input in ["İSTANBUL", "istanbul", "Istanbul", "ıstanbul"] {
            assert_eq!(fold(input), "istanbul", "input: {input}");
        }
    }

    #[test]
    fn fold_strips_diacritics_and_trims() {
        assert_eq!(fold("Değil"), "degil");
        assert_eq!(fold("  Şey "), "sey");
    }

    #[test]
    fn normalize_builds_matching_key() {
        assert_eq!(normalize("Altyazı: M.K."), "altyazi m k");
        assert_eq!(normalize("snake_case--word!!"), "snake case word");
        assert_eq!(normalize("♪♪ ..."), "");
    }
}
