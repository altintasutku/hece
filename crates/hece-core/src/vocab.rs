//! Vocabulary file parsing and budgeted Whisper prompt rendering.

use std::{collections::HashSet, sync::LazyLock};

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::text::fold;

/// Default token budget for the vocabulary prompt.
pub const DEFAULT_PROMPT_TOKENS: usize = 200;
/// Hard cap on prompt tokens (Whisper's initial prompt limit).
pub const MAX_PROMPT_TOKENS: usize = 223;
/// Maximum length of a single term, in chars.
pub const MAX_TERM_CHARS: usize = 40;
/// Default text placed before the term list in the prompt.
pub const DEFAULT_PROMPT_PREFIX: &str = "Vocabulary: ";

/// Ordered list of custom terms; earlier terms have higher priority.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Vocabulary {
    /// Terms in priority order. Expected to come from [`Vocabulary::parse`] (already
    /// cleaned); constructing this directly or via `Deserialize` bypasses [`clean_term`].
    pub terms: Vec<String>,
}

/// Cleans a raw term; `None` if it is not a usable vocabulary term.
#[must_use]
pub fn clean_term(raw: &str) -> Option<String> {
    let visible = INVISIBLE.replace_all(raw, "");
    let collapsed = visible.split_whitespace().collect::<Vec<_>>().join(" ");
    let term = collapsed.trim_matches(EDGE_CHARS);
    let valid = !term.is_empty()
        && !term.contains(',')
        && term.chars().count() <= MAX_TERM_CHARS
        && term.chars().any(char::is_alphanumeric);
    valid.then(|| term.to_owned())
}

const EDGE_CHARS: &[char] = &[
    ' ', ',', '.', ';', ':', '!', '?', '"', '\'', '\u{201C}', '\u{201D}', '(', ')', '[', ']',
];

/// Every Unicode general category C char (control, format, surrogate, private use, unassigned).
#[expect(
    clippy::expect_used,
    reason = "static regex built from a literal; invalid literal is a programmer error"
)]
static INVISIBLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\p{C}").expect("valid regex literal"));

/// Whether `c` ends a line in Python's `str.splitlines()`.
fn is_line_break(c: char) -> bool {
    matches!(
        c,
        '\n' | '\r'
            | '\u{0B}'
            | '\u{0C}'
            | '\u{1C}'
            | '\u{1D}'
            | '\u{1E}'
            | '\u{85}'
            | '\u{2028}'
            | '\u{2029}'
    )
}

/// Strips one leading `-`, `*` or `+` bullet followed by whitespace.
fn strip_bullet(line: &str) -> &str {
    let mut chars = line.chars();
    match (chars.next(), chars.next()) {
        (Some('-' | '*' | '+'), Some(next)) if next.is_whitespace() => chars.as_str().trim_start(),
        _ => line,
    }
}

impl Vocabulary {
    /// Parses a vocabulary file: one term per line, `#` comments, `!term` exclusions.
    #[must_use]
    pub fn parse(text: &str) -> Vocabulary {
        let mut candidates: Vec<(String, String)> = Vec::new();
        let mut excluded: HashSet<String> = HashSet::new();
        for line in text.split(is_line_break) {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let entry = strip_bullet(line);
            if let Some(rest) = entry.strip_prefix('!') {
                if let Some(term) = clean_term(rest) {
                    excluded.insert(fold(&term));
                }
            } else if let Some(term) = clean_term(entry) {
                let key = fold(&term);
                candidates.push((key, term));
            }
        }
        let mut seen: HashSet<String> = HashSet::new();
        let terms = candidates
            .into_iter()
            .filter(|(key, _)| !excluded.contains(key) && seen.insert(key.clone()))
            .map(|(_, term)| term)
            .collect();
        Vocabulary { terms }
    }

    /// Renders the longest prefix of terms whose prompt fits `max_tokens`.
    #[must_use]
    pub fn whisper_prompt(
        &self,
        prefix: &str,
        count: &dyn Fn(&str) -> usize,
        max_tokens: usize,
    ) -> Option<String> {
        let budget = max_tokens.min(MAX_PROMPT_TOKENS);
        let mut best: Option<String> = None;
        for n in 1..=self.terms.len() {
            let rendered = format!("{prefix}{}.", self.terms[..n].join(", "));
            // faster-whisper prepends a space to the initial prompt.
            if count(&format!(" {rendered}")) > budget {
                break;
            }
            best = Some(rendered);
        }
        best
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(terms: &[&str]) -> Vocabulary {
        Vocabulary {
            terms: terms.iter().map(|s| (*s).to_owned()).collect(),
        }
    }

    #[test]
    fn parse_skips_comments_blanks_and_strips_bullets() {
        let vocab =
            Vocabulary::parse("# comment\n\n- Kubernetes\n* PostgreSQL\n+ Tauri\n  İstanbul  \n");
        assert_eq!(
            vocab.terms,
            ["Kubernetes", "PostgreSQL", "Tauri", "İstanbul"]
        );
    }

    #[test]
    fn parse_dedupes_case_insensitively_first_wins() {
        let vocab = Vocabulary::parse("Rust\nrust\nRUST\n");
        assert_eq!(vocab.terms, ["Rust"]);
    }

    #[test]
    fn parse_exclusion_later_removes_earlier_term() {
        let vocab = Vocabulary::parse("Tauri\nRust\n!Tauri\n- !rust\n");
        assert_eq!(vocab.terms, [] as [String; 0]);
        assert_eq!(Vocabulary::parse("!Tauri\nTauri\nRust").terms, ["Rust"]);
    }

    #[test]
    fn parse_drops_invalid_terms() {
        let long = "a".repeat(41);
        let vocab = Vocabulary::parse(&format!("a, b\n{long}\n...\nOk\n"));
        assert_eq!(vocab.terms, ["Ok"]);
        assert!(clean_term(&"a".repeat(40)).is_some());
    }

    #[test]
    fn parse_cleans_quotes_and_invisible_chars() {
        let vocab = Vocabulary::parse("  \"Kubernetes\",  \nPost\u{200B}greSQL\n");
        assert_eq!(vocab.terms, ["Kubernetes", "PostgreSQL"]);
    }

    #[test]
    fn parse_empty_is_empty() {
        assert_eq!(Vocabulary::parse("").terms, [] as [String; 0]);
    }

    #[test]
    fn prompt_renders_terms() {
        let count = |s: &str| s.len();
        assert_eq!(
            v(&["Rust", "Tauri"]).whisper_prompt("Notes: ", &count, 1000),
            Some("Notes: Rust, Tauri.".to_owned())
        );
    }

    #[test]
    fn prompt_budget_boundary_is_inclusive() {
        let count = |s: &str| s.len();
        let budget = " Notes: Rust.".len();
        assert_eq!(
            v(&["Rust", "Tauri"]).whisper_prompt("Notes: ", &count, budget),
            Some("Notes: Rust.".to_owned())
        );
    }

    #[test]
    fn prompt_none_when_first_term_too_long_or_empty() {
        let count = |s: &str| s.len();
        assert_eq!(
            v(&["Kubernetes"]).whisper_prompt("Notes: ", &count, 10),
            None
        );
        assert_eq!(
            Vocabulary::default().whisper_prompt("Notes: ", &count, 1000),
            None
        );
    }

    #[test]
    fn prompt_budget_is_clamped_to_hard_cap() {
        let count = |s: &str| s.len();
        let terms: Vec<String> = (0..60).map(|i| format!("term{i:03}")).collect();
        let vocab = Vocabulary { terms };
        let prompt = vocab
            .whisper_prompt(DEFAULT_PROMPT_PREFIX, &count, 10_000)
            .unwrap();
        let len = format!(" {prompt}").len();
        // Each term adds 9 bytes, so a correct clamp lands in 215..=223.
        assert!(len <= MAX_PROMPT_TOKENS, "len {len}");
        assert!(len > 214, "len {len}");
    }

    #[test]
    fn clean_term_drops_all_format_and_tag_chars() {
        assert_eq!(clean_term("Ru\u{2066}st").as_deref(), Some("Rust"));
        assert_eq!(clean_term("Rust\u{E0041}").as_deref(), Some("Rust"));
        assert_eq!(clean_term("Ru\u{00AD}st").as_deref(), Some("Rust"));
        assert_eq!(clean_term("Ru\u{200B}st").as_deref(), Some("Rust"));
        assert_eq!(clean_term("Ru\tst").as_deref(), Some("Rust"));
    }

    #[test]
    fn parse_splits_like_python_splitlines() {
        for input in [
            "Rust\rTauri",
            "Rust\u{2028}Tauri",
            "Rust\r\nTauri",
            "Rust\u{85}Tauri",
            "Rust\u{0B}Tauri",
            "Rust\u{1C}Tauri",
            "Rust\u{2029}Tauri",
        ] {
            assert_eq!(
                Vocabulary::parse(input).terms,
                ["Rust", "Tauri"],
                "{input:?}"
            );
        }
    }
}
