//! Benchmark metrics: word/char error rates, vocabulary hits, and long-form checks.
//!
//! Everything here is pure. Reference and hypothesis texts are only held in memory by
//! the caller; nothing in this module logs or prints them.

use std::collections::HashMap;
use std::fmt;
use std::sync::LazyLock;

use regex::Regex;

use crate::guard::is_negation_token;
use crate::text::fold;
use crate::textnorm::normalize_words;

/// Minimum length of a passage of deleted words that counts as a dropped passage.
const DELETED_RUN_MIN: usize = 5;
/// Minimum words for a sentence to count in the repetition check.
const SENTENCE_MIN_WORDS: usize = 4;
/// Word n-gram length of the repeated-sequence check.
const NGRAM_LEN: usize = 5;

#[expect(
    clippy::expect_used,
    reason = "static regex built from a literal; invalid literal is a programmer error"
)]
static SENTENCE_END: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[.!?…]+(?:\s+|$)").expect("valid regex literal"));

/// Levenshtein distance between two sequences (insert, delete, substitute all cost 1).
#[must_use]
pub fn edit_distance<T: PartialEq>(r: &[T], h: &[T]) -> usize {
    let mut prev: Vec<usize> = (0..=h.len()).collect();
    for (i, rx) in r.iter().enumerate() {
        let mut cur = Vec::with_capacity(h.len() + 1);
        cur.push(i + 1);
        for (j, hx) in h.iter().enumerate() {
            let sub = prev[j] + usize::from(rx != hx);
            cur.push(sub.min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[h.len()]
}

// Counts are far below 2^52, so the usize -> f64 conversion is exact.
#[expect(
    clippy::cast_precision_loss,
    reason = "counts are far below 2^52, so the conversion is exact"
)]
fn ratio(num: usize, den: usize) -> f64 {
    num as f64 / den.max(1) as f64
}

/// Word error rate on normalized words: edit distance / max(reference words, 1).
#[must_use]
pub fn word_error_rate(r: &str, h: &str) -> f64 {
    let (r, h) = (normalize_words(r), normalize_words(h));
    ratio(edit_distance(&r, &h), r.len())
}

/// Character error rate on the normalized words joined by single spaces.
#[must_use]
pub fn char_error_rate(r: &str, h: &str) -> f64 {
    let r: Vec<char> = normalize_words(r).join(" ").chars().collect();
    let h: Vec<char> = normalize_words(h).join(" ").chars().collect();
    ratio(edit_distance(&r, &h), r.len())
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Whether `term` occurs in `text` at a word start (folded, `-` counts as a space).
///
/// Suffixed forms match (`Berre'ye`), mid-word occurrences do not (`TL` in `kitle`).
#[must_use]
pub fn contains(text: &str, term: &str) -> bool {
    let needle = fold(term).replace('-', " ");
    let hay = fold(text).replace('-', " ");
    let mut prev: Option<char> = None;
    for (i, c) in hay.char_indices() {
        if prev.is_none_or(|p| !is_word_char(p)) && hay[i..].starts_with(&needle) {
            return true;
        }
        prev = Some(c);
    }
    // An empty needle also matches at the end of the text.
    needle.is_empty() && prev.is_none_or(|p| !is_word_char(p))
}

/// One successfully transcribed file. Text is held in memory only; never log it.
#[derive(Clone)]
pub struct Scored {
    /// File name shown in reports.
    pub file: String,
    /// Reference text.
    pub reference: String,
    /// Transcribed text.
    pub hypothesis: String,
    /// Wall-clock transcription time in seconds.
    pub seconds: f64,
}

impl fmt::Debug for Scored {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Scored")
            .field("file", &self.file)
            .field("reference_len", &self.reference.len())
            .field("hypothesis_len", &self.hypothesis.len())
            .field("seconds", &self.seconds)
            .finish()
    }
}

/// Aggregate numbers for the short-utterance set. Means are over scored files.
#[derive(Debug, Clone, PartialEq)]
pub struct ShortReport {
    /// Scored files.
    pub files: usize,
    /// Files whose transcription failed (excluded from the means).
    pub failed: usize,
    /// Mean word error rate.
    pub wer: f64,
    /// Mean character error rate.
    pub cer: f64,
    /// Target vocabulary terms found in the hypotheses.
    pub names_hit: usize,
    /// Target vocabulary terms present in the references.
    pub names_total: usize,
    /// Vocabulary terms in a hypothesis but not in its reference.
    pub invented: usize,
    /// Mean transcription seconds per scored file.
    pub seconds_per_utterance: f64,
}

fn mean(sum: f64, n: usize) -> f64 {
    #[expect(clippy::cast_precision_loss, reason = "file counts are far below 2^52")]
    let n = n.max(1) as f64;
    sum / n
}

/// Scores the short-utterance set. `failed` is the number of files that errored.
#[must_use]
pub fn score_short(rows: &[Scored], failed: usize, terms: &[String]) -> ShortReport {
    let (mut wer, mut cer, mut seconds) = (0.0, 0.0, 0.0);
    let (mut names_hit, mut names_total, mut invented) = (0, 0, 0);
    for row in rows {
        wer += word_error_rate(&row.reference, &row.hypothesis);
        cer += char_error_rate(&row.reference, &row.hypothesis);
        seconds += row.seconds;
        for term in terms {
            let in_ref = contains(&row.reference, term);
            let in_hyp = contains(&row.hypothesis, term);
            if in_ref {
                names_total += 1;
                names_hit += usize::from(in_hyp);
            } else if in_hyp {
                invented += 1;
            }
        }
    }
    ShortReport {
        files: rows.len(),
        failed,
        wer: mean(wer, rows.len()),
        cer: mean(cer, rows.len()),
        names_hit,
        names_total,
        invented,
        seconds_per_utterance: mean(seconds, rows.len()),
    }
}

/// Aggregate numbers for the long-recording set.
#[derive(Debug, Clone, PartialEq)]
pub struct LongReport {
    /// The same numbers as the short set.
    pub short: ShortReport,
    /// Dropped passages of at least five reference words; deletion runs separated by a
    /// single matched or substituted word count as one passage.
    pub deleted_runs: usize,
    /// Hypothesis sentences repeated more often than in the reference.
    pub repeated_sentences: usize,
    /// Distinct word 5-grams repeated in the hypothesis (at least twice) more often than in
    /// the reference: catches unpunctuated loops and chunk-overlap duplication.
    pub repeated_ngrams: usize,
    /// Reference negation tokens that the alignment deletes or replaces with a non-negation
    /// word. Word-level negations only (değil, yok, hayır, asla, hiç…, not, never, n't…);
    /// Turkish verb negation (-me/-ma) is not detected.
    pub dropped_negations: usize,
}

/// Scores the long-recording set.
#[must_use]
pub fn score_long(rows: &[Scored], failed: usize, terms: &[String]) -> LongReport {
    let (mut runs, mut repeated, mut ngrams, mut dropped) = (0, 0, 0, 0);
    for row in rows {
        let (r, h) = (
            normalize_words(&row.reference),
            normalize_words(&row.hypothesis),
        );
        let ops = align(&r, &h);
        runs += deleted_runs_in(&ops, DELETED_RUN_MIN);
        repeated += repeated_sentences(&row.reference, &row.hypothesis);
        ngrams += repeated_ngrams(&r, &h, NGRAM_LEN);
        dropped += dropped_negations(&ops, &r, &h);
    }
    LongReport {
        short: score_short(rows, failed, terms),
        deleted_runs: runs,
        repeated_sentences: repeated,
        repeated_ngrams: ngrams,
        dropped_negations: dropped,
    }
}

/// One step of a word alignment; indices point into the reference (`r`) and hypothesis (`h`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlignOp {
    /// Equal words.
    Match {
        /// Reference index.
        r: usize,
        /// Hypothesis index.
        h: usize,
    },
    /// Reference word replaced by a different hypothesis word.
    Sub {
        /// Reference index.
        r: usize,
        /// Hypothesis index.
        h: usize,
    },
    /// Reference word missing from the hypothesis.
    Del {
        /// Reference index.
        r: usize,
    },
    /// Hypothesis word absent from the reference.
    Ins {
        /// Hypothesis index.
        h: usize,
    },
}

/// Word alignment in reference order, from a backtrace of the edit-distance table. On ties
/// it prefers match/substitution, then deletion, then insertion.
#[must_use]
pub fn align(r: &[String], h: &[String]) -> Vec<AlignOp> {
    let mut table = vec![vec![0usize; h.len() + 1]; r.len() + 1];
    for (i, row) in table.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in table[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=r.len() {
        for j in 1..=h.len() {
            let sub = table[i - 1][j - 1] + usize::from(r[i - 1] != h[j - 1]);
            table[i][j] = sub.min(table[i - 1][j] + 1).min(table[i][j - 1] + 1);
        }
    }

    let (mut i, mut j) = (r.len(), h.len());
    let mut ops = Vec::with_capacity(r.len().max(h.len()));
    while i > 0 || j > 0 {
        let diag = i > 0
            && j > 0
            && table[i][j] == table[i - 1][j - 1] + usize::from(r[i - 1] != h[j - 1]);
        if diag {
            let (ri, hi) = (i - 1, j - 1);
            ops.push(if r[ri] == h[hi] {
                AlignOp::Match { r: ri, h: hi }
            } else {
                AlignOp::Sub { r: ri, h: hi }
            });
            i -= 1;
            j -= 1;
        } else if i > 0 && table[i][j] == table[i - 1][j] + 1 {
            ops.push(AlignOp::Del { r: i - 1 });
            i -= 1;
        } else {
            ops.push(AlignOp::Ins { h: j - 1 });
            j -= 1;
        }
    }
    ops.reverse();
    ops
}

/// Counts dropped passages of at least `min_len` deleted reference words.
///
/// Deletion runs separated by at most one matched or substituted reference word merge into
/// one passage; only the deleted words count toward its length. Insertions do not split it.
#[must_use]
pub fn deleted_runs(r: &[String], h: &[String], min_len: usize) -> usize {
    deleted_runs_in(&align(r, h), min_len)
}

fn deleted_runs_in(ops: &[AlignOp], min_len: usize) -> usize {
    let (mut passage, mut gap, mut runs) = (0usize, 0usize, 0usize);
    for op in ops {
        match op {
            AlignOp::Del { .. } => {
                if gap > 1 && passage >= min_len {
                    runs += 1;
                }
                passage = if gap > 1 { 1 } else { passage + 1 };
                gap = 0;
            }
            AlignOp::Match { .. } | AlignOp::Sub { .. } => gap += 1,
            AlignOp::Ins { .. } => {}
        }
    }
    runs + usize::from(passage >= min_len)
}

fn is_negation_word(word: &str) -> bool {
    is_negation_token(&fold(word))
}

/// Reference negation tokens that the alignment deletes or substitutes with a non-negation.
fn dropped_negations(ops: &[AlignOp], r: &[String], h: &[String]) -> usize {
    ops.iter()
        .filter(|op| match **op {
            AlignOp::Del { r: ri } => is_negation_word(&r[ri]),
            AlignOp::Sub { r: ri, h: hi } => is_negation_word(&r[ri]) && !is_negation_word(&h[hi]),
            AlignOp::Match { .. } | AlignOp::Ins { .. } => false,
        })
        .count()
}

fn ngram_counts(words: &[String], n: usize) -> HashMap<&[String], usize> {
    let mut counts = HashMap::new();
    for gram in words.windows(n) {
        *counts.entry(gram).or_insert(0) += 1;
    }
    counts
}

/// Distinct word `n`-grams that occur at least twice in `h` and more often than in `r`.
/// Requiring a repeat keeps plain substitutions (new n-grams seen once) from counting.
#[must_use]
pub fn repeated_ngrams(r: &[String], h: &[String], n: usize) -> usize {
    if n == 0 {
        return 0;
    }
    let reference = ngram_counts(r, n);
    ngram_counts(h, n)
        .into_iter()
        .filter(|(gram, count)| *count >= 2 && *count > reference.get(gram).copied().unwrap_or(0))
        .count()
}

fn sentence_counts(text: &str) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    for sentence in SENTENCE_END.split(text) {
        let words = normalize_words(sentence);
        if words.len() >= SENTENCE_MIN_WORDS {
            *counts.entry(words.join(" ")).or_insert(0) += 1;
        }
    }
    counts
}

/// Counts distinct hypothesis sentences (at least four words) that occur at least twice in
/// the hypothesis and more often than their word sequence occurs in the reference. The
/// reference side is matched on normalized words, so it does not depend on its punctuation.
#[must_use]
pub fn repeated_sentences(r: &str, h: &str) -> usize {
    let reference = normalize_words(r);
    sentence_counts(h)
        .into_iter()
        .filter(|(sentence, n)| {
            *n >= 2 && {
                let needle: Vec<&str> = sentence.split(' ').collect();
                *n > occurrences(&reference, &needle)
            }
        })
        .count()
}

/// Non-overlapping occurrences of `needle` as a contiguous word sequence in `words`.
fn occurrences(words: &[String], needle: &[&str]) -> usize {
    if needle.is_empty() || needle.len() > words.len() {
        return 0;
    }
    let mut count = 0;
    let mut i = 0;
    while i + needle.len() <= words.len() {
        if words[i..i + needle.len()]
            .iter()
            .zip(needle)
            .all(|(w, n)| w == n)
        {
            count += 1;
            i += needle.len();
        } else {
            i += 1;
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(file: &str, reference: &str, hypothesis: &str) -> Scored {
        Scored {
            file: file.to_owned(),
            reference: reference.to_owned(),
            hypothesis: hypothesis.to_owned(),
            seconds: 1.0,
        }
    }

    fn words(n: usize, prefix: &str) -> Vec<String> {
        (0..n).map(|i| format!("{prefix}{i}")).collect()
    }

    #[test]
    fn wer_edge_cases() {
        assert!((word_error_rate("", "") - 0.0).abs() < 1e-12);
        assert!((word_error_rate("a b", "") - 1.0).abs() < 1e-12);
        assert!((word_error_rate("", "x") - 1.0).abs() < 1e-12);
        assert!((word_error_rate("bu bir test", "Bu bir test.") - 0.0).abs() < 1e-12);
        assert!((word_error_rate("bir iki üç dört", "bir iki üç beş") - 0.25).abs() < 1e-12);
    }

    #[test]
    fn cer_by_hand() {
        // "abc def" vs "abc dxf": one substitution over 7 chars.
        assert!((char_error_rate("abc def", "abc dxf") - 1.0 / 7.0).abs() < 1e-12);
        assert!((char_error_rate("", "") - 0.0).abs() < 1e-12);
        assert!((char_error_rate("ab", "") - 1.0).abs() < 1e-12);
    }

    #[test]
    fn edit_distance_basic() {
        assert_eq!(edit_distance(&[1, 2, 3], &[1, 3]), 1);
        assert_eq!(edit_distance::<u8>(&[], &[]), 0);
        assert_eq!(edit_distance(&['a', 'b'], &['b', 'a']), 2);
    }

    #[test]
    fn contains_matches_word_start_only() {
        assert!(contains("Berre'ye geldi", "Berre"));
        assert!(!contains("kitle", "TL"));
        assert!(contains("Wi Fi açık", "Wi-Fi"));
        assert!(contains("wi-fi açık", "Wi-Fi"));
        assert!(contains("C++ yaz", "C++"));
        assert!(contains("ESP32 kartı", "esp32"));
        assert!(contains("İstanbul'da", "istanbul"));
        assert!(!contains("aksam", "ksam"));
        assert!(!contains("anything", "(["));
    }

    #[test]
    fn short_report_aggregates_names_and_invented() {
        let terms: Vec<String> = ["Berre", "ESP32", "Arduino"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        let rows = vec![
            // Berre targeted and hit; Arduino invented.
            row("a", "Berre geldi", "Berre geldi Arduino"),
            // ESP32 targeted and missed.
            row("b", "ESP32 kartı", "kart kartı"),
            // No targets; nothing invented.
            row("c", "merhaba dünya", "merhaba dünya"),
        ];
        let report = score_short(&rows, 2, &terms);
        assert_eq!(report.files, 3);
        assert_eq!(report.failed, 2);
        assert_eq!(report.names_total, 2);
        assert_eq!(report.names_hit, 1);
        assert_eq!(report.invented, 1);
        assert!((report.seconds_per_utterance - 1.0).abs() < 1e-12);
        assert!(report.wer > 0.0 && report.cer > 0.0);
    }

    #[test]
    fn short_report_empty_has_no_nan() {
        let report = score_short(&[], 1, &[]);
        assert_eq!(report.files, 0);
        assert_eq!(report.failed, 1);
        assert!(report.wer == 0.0 && report.cer == 0.0 && report.seconds_per_utterance == 0.0);
    }

    #[test]
    fn deleted_runs_counts_long_deletions_only() {
        let r = words(6, "w");
        assert_eq!(deleted_runs(&r, &[], 5), 1);
        let mut r2 = words(4, "a");
        r2.extend(["m1".to_owned(), "m2".to_owned()]);
        r2.extend(words(4, "b"));
        let kept = ["m1".to_owned(), "m2".to_owned()];
        assert_eq!(deleted_runs(&r2, &kept, 5), 0);
        assert_eq!(deleted_runs(&r2, &kept, 4), 2);
    }

    #[test]
    fn deleted_runs_merges_runs_split_by_one_matched_word() {
        let r = normalize_words("sonra bir gün bir adam bir köye gitti ve orada kaldı");
        let h = normalize_words("sonra bir kaldı");
        assert_eq!(deleted_runs(&r, &h, 5), 1);
    }

    #[test]
    fn dropped_negations_ignores_unrelated_insertions() {
        let rows = vec![row(
            "a",
            "bu doğru değil ama sonra hiç gelmedi",
            "bu doğru ama sonra hiç gelmedi yok",
        )];
        assert_eq!(score_long(&rows, 0, &[]).dropped_negations, 1);
        let kept = vec![row("a", "bu doğru değil", "bu doğru değil")];
        assert_eq!(score_long(&kept, 0, &[]).dropped_negations, 0);
    }

    #[test]
    fn repeated_ngrams_flags_extra_hypothesis_sequences() {
        let r = "sabah eve gittik ertesi gün işe başladık";
        let h = "sabah eve gittik ertesi gün geldik eve gittik ertesi gün geldik";
        let rows = vec![row("a", r, h)];
        assert!(score_long(&rows, 0, &[]).repeated_ngrams >= 1);
        let same = vec![row("a", r, r)];
        assert_eq!(score_long(&same, 0, &[]).repeated_ngrams, 0);
        // Substitutions create new n-grams once each; they are not repeats.
        let subbed = vec![row("a", r, "sabah eve döndük ertesi gün okula başladık")];
        assert_eq!(score_long(&subbed, 0, &[]).repeated_ngrams, 0);
    }

    #[test]
    fn repeated_sentences_counts_extra_hypothesis_copies() {
        let s = "bu cümle dört kelime taşır.";
        let r = format!("{s} Başka bir cümle burada.");
        let h = format!("{s} {s} Başka bir cümle burada.");
        assert_eq!(repeated_sentences(&r, &h), 1);
        assert_eq!(repeated_sentences(&r, &r), 0);
        // The reference's punctuation does not matter: same words, no full stops.
        assert_eq!(
            repeated_sentences(
                "bu cümle dört kelime taşır ve biter",
                &format!("{s} ve biter.")
            ),
            0
        );
        // A sentence said twice in the reference and twice in the hypothesis is not a repeat.
        let twice = format!("{s} {s}");
        assert_eq!(repeated_sentences(&twice.replace('.', ""), &twice), 0);
        // Sentences under four words never count.
        assert_eq!(repeated_sentences("tamam.", "tamam. tamam. tamam."), 0);
    }

    #[test]
    fn long_report_counts_dropped_negations() {
        let rows = vec![row("a", "bu doğru değil", "bu doğru")];
        let long = score_long(&rows, 0, &[]);
        assert_eq!(long.dropped_negations, 1);
        assert_eq!(long.short.files, 1);
    }

    #[test]
    fn scored_debug_hides_text() {
        let shown = format!("{:?}", row("a.wav", "gizli metin", "gizli"));
        assert!(!shown.contains("gizli"));
        assert!(shown.contains("a.wav"));
    }
}
