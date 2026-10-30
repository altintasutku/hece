//! Drops known Whisper hallucinations (subtitle credits, outro phrases, annotations).

use std::sync::LazyLock;

use regex::Regex;

use crate::{text::normalize, types::Segment};

/// Segments with `no_speech_prob` strictly above this and few words are dropped.
pub const NO_SPEECH_THRESHOLD: f32 = 0.6;
/// Maximum word count for a segment to be dropped on `no_speech_prob` alone.
pub const SHORT_SEGMENT_MAX_WORDS: usize = 3;

const PHRASES: &[&str] = &[
    "İzlediğiniz için teşekkürler",
    "İzlediğiniz için teşekkür ederim",
    "İzlediğiniz için teşekkür ederiz",
    "Abone olmayı unutmayın",
    "Abone olun",
    "Kanalıma abone olmayı unutmayın",
    "Kanalımıza abone olmayı unutmayın",
    "Beğenmeyi ve abone olmayı unutmayın",
    "Abone olmayı ve beğenmeyi unutmayın",
    "Videoyu beğenmeyi unutmayın",
    "Bir sonraki videoda görüşmek üzere",
    "Bir sonraki videoda görüşürüz",
    "Altyazı",
    "Thanks for watching",
    "Thank you for watching",
    "Thank you so much for watching",
    "Please subscribe",
    "Subscribe to my channel",
    "Like and subscribe",
    "Don't forget to like and subscribe",
    "The end",
];

const CREDIT_PATTERNS: &[&str] = &[
    r"\baltyazi\s+m\s*k\b",
    r"\baltyazilar?\b.*\btarafindan\b",
    r"\bamara\s+org\b",
    r"\bsous titres\b",
    r"\buntertitel\b",
];

// Invariant: the phrase list and patterns above are literals; a bad one is a programmer error.
#[expect(
    clippy::expect_used,
    reason = "regex built from the static phrase list; invalid list is a programmer error"
)]
static REPEAT_MATCHER: LazyLock<Regex> = LazyLock::new(|| {
    let mut keys: Vec<String> = PHRASES.iter().map(|p| normalize(p)).collect();
    keys.sort();
    keys.dedup();
    keys.sort_by_key(|k| std::cmp::Reverse(k.len()));
    let alternation: Vec<String> = keys.iter().map(|k| regex::escape(k)).collect();
    Regex::new(&format!(r"^(?:(?:{})\s*)+$", alternation.join("|")))
        .expect("valid repeat matcher built from static phrases")
});

#[expect(
    clippy::expect_used,
    reason = "static regex literals; invalid literal is a programmer error"
)]
static CREDITS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    CREDIT_PATTERNS
        .iter()
        .map(|p| Regex::new(p).expect("valid regex literal"))
        .collect()
});

#[expect(
    clippy::expect_used,
    reason = "static regex built from a literal; invalid literal is a programmer error"
)]
static ANNOTATION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)^\s*([\[(*♪]).*([\])*♪])\s*$").expect("valid regex literal"));

/// Result of filtering a segment list.
#[derive(Debug, Clone, PartialEq)]
pub struct Filtered {
    /// Concatenated text of the kept segments, trimmed.
    pub text: String,
    /// Segments that survived.
    pub kept: Vec<Segment>,
    /// Number of dropped segments.
    pub dropped: usize,
}

/// Whether the whole text is a known Whisper hallucination (annotation, outro phrase or credit).
#[must_use]
pub fn is_known_hallucination(text: &str) -> bool {
    if ANNOTATION.is_match(text) {
        return true;
    }
    let key = normalize(text);
    key.is_empty() || REPEAT_MATCHER.is_match(&key) || CREDITS.iter().any(|re| re.is_match(&key))
}

fn should_drop(seg: &Segment) -> bool {
    if is_known_hallucination(&seg.text) {
        return true;
    }
    seg.no_speech_prob > NO_SPEECH_THRESHOLD
        && normalize(&seg.text).split_whitespace().count() <= SHORT_SEGMENT_MAX_WORDS
}

/// Drops hallucinated and likely-silent segments and rebuilds the text from the rest.
#[must_use]
pub fn filter_segments(segs: Vec<Segment>) -> Filtered {
    let total = segs.len();
    let kept: Vec<Segment> = segs.into_iter().filter(|s| !should_drop(s)).collect();
    let text = kept.iter().map(|s| s.text.as_str()).collect::<String>();
    Filtered {
        text: text.trim().to_owned(),
        dropped: total - kept.len(),
        kept,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(text: &str, no_speech: f32, logprob: f32, start: f32, end: f32) -> Segment {
        Segment {
            text: text.to_owned(),
            start_s: start,
            end_s: end,
            no_speech_prob: no_speech,
            avg_logprob: logprob,
        }
    }

    #[test]
    fn known_hallucinations_are_detected() {
        for t in [
            " İzlediğiniz için teşekkürler.",
            " Abone olun. Abone olun.",
            " Thanks for watching!",
            " Altyazı M.K.",
            " altyazılar X tarafından",
            " www.amara.org",
            " [Music]",
            " (applause)",
            " ♪ la la ♪",
            " ...",
        ] {
            assert!(is_known_hallucination(t), "should drop {t:?}");
        }
    }

    #[test]
    fn real_sentences_are_kept() {
        assert!(!is_known_hallucination(
            " Abone olun dedi ama kimse gelmedi."
        ));
        assert!(!is_known_hallucination(
            " Thanks for watching the build, it passed."
        ));
    }

    #[test]
    fn short_segment_no_speech_boundary() {
        let dropped = filter_segments(vec![seg(" bir iki üç", 0.7, -0.1, 0.0, 1.0)]);
        assert_eq!(dropped.dropped, 1);
        let four = filter_segments(vec![seg(" bir iki üç dört", 0.7, -0.1, 0.0, 1.0)]);
        assert_eq!(four.dropped, 0);
        let strict = filter_segments(vec![seg(" bir iki üç", 0.6, -0.1, 0.0, 1.0)]);
        assert_eq!(strict.dropped, 0);
    }

    #[test]
    fn empty_input() {
        let f = filter_segments(vec![]);
        assert_eq!(f.text, "");
        assert_eq!(f.dropped, 0);
        assert_eq!(f.kept, [] as [Segment; 0]);
    }

    #[test]
    fn multiline_annotation_is_dropped() {
        assert!(is_known_hallucination(" [Music\nplaying]"));
    }

    #[test]
    fn mixed_list_drops_short_no_speech_segment() {
        let f = filter_segments(vec![
            seg(" Merhaba dünya.", 0.0, -0.1, 0.0, 1.0),
            seg(" evet tamam", 0.8, -0.1, 1.0, 2.0),
            seg(" Nasılsın?", 0.1, -0.1, 2.0, 3.0),
        ]);
        assert_eq!(f.text, "Merhaba dünya. Nasılsın?");
        assert_eq!(f.dropped, 1);
        assert_eq!(f.kept.len(), 2);
    }

    #[test]
    fn mixed_list_joins_kept_text() {
        let f = filter_segments(vec![
            seg(" Merhaba dünya.", 0.0, -0.1, 0.0, 1.0),
            seg(" [Music]", 0.0, -0.1, 1.0, 2.0),
            seg(" Nasılsın?", 0.0, -0.1, 2.0, 3.0),
        ]);
        assert_eq!(f.text, "Merhaba dünya. Nasılsın?");
        assert_eq!(f.dropped, 1);
        assert_eq!(f.kept.len(), 2);
    }
}
