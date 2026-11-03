//! Drops known Whisper hallucinations (subtitle credits, outro phrases, annotations).
//!
//! On silence or noise Whisper emits stock lines learned from subtitled videos. A segment
//! is dropped only when its *whole* text is such a line (or several of them back to back)
//! or matches a subtitle-credit pattern; a phrase inside a real sentence is kept. Bare
//! "thank you" words (`Gracias`, `Merci`, `Danke`, `Спасибо`, `감사합니다`, …) are never
//! listed, because they are plausible dictations.
//!
//! Coverage: Turkish, English, Spanish, German, French, Portuguese, Italian, Dutch, Polish,
//! Russian, Ukrainian, Japanese, Chinese, Korean, Arabic and Indonesian. Hindi has no
//! entries: the public lists only show English outros for it, which are already covered.
//! Every non-Turkish/English entry comes from a public source:
//!
//! - `[928]` openai/whisper discussion #928 (Amara.org, ZDF, WDR/SWR, `QTSS`, `ST'501` credits)
//! - `[2608]` openai/whisper discussion #2608 (Arabic "Nancy Qunqar" credit)
//! - `[hf]` Hugging Face dataset `sachaarbonel/whisper-hallucinations` (Whisper run on
//!   noise-only clips, phrases per language with counts)
//! - `[vexa]` Vexa-ai/vexa `hallucinations/*.txt` (curated es/pt/ru/ja lists from production)
//! - `[st]` Dadangdut33/Speech-Translate `base_hallucination_filter.json`
//! - `[tacit]` sangmin7648/tacit issue #14 (Korean, measured over 24 h of ko dictation)
//! - `[stt46]` jianchang512/stt issue #46 (Chinese `明镜与点点` sign-off on faster-whisper)

use std::sync::LazyLock;

use regex::Regex;

use crate::{
    text::{fold, normalize},
    types::Segment,
};

/// Segments with `no_speech_prob` strictly above this and few words are dropped.
pub const NO_SPEECH_THRESHOLD: f32 = 0.6;
/// Maximum word count for a segment to be dropped on `no_speech_prob` alone.
///
/// In scripts written without spaces (Chinese, Japanese, Thai, …) each character counts as
/// a word, so a long unspaced sentence is not mistaken for a short one.
pub const SHORT_SEGMENT_MAX_WORDS: usize = 3;

/// Whole-transcript outro phrases, compared after [`normalize`].
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
    // es [hf] [vexa]
    "Gracias por ver el video",
    "Gracias por ver este video",
    "Suscríbete al canal",
    "Suscríbete a mi canal",
    "Suscríbete al canal y dale a like",
    "Dale like y suscríbete",
    "No olvides suscribirte",
    "Nos vemos en el próximo video",
    // de [hf]
    "Danke fürs Zuschauen",
    "Danke fürs Ansehen",
    "Bitte abonnieren",
    "Abonniere meinen Kanal",
    // fr [hf] [928]
    "Merci d'avoir regardé",
    "Merci d'avoir regardé cette vidéo",
    "Merci d'avoir regardé la vidéo",
    "Abonnez-vous à ma chaîne",
    "N'oubliez pas de vous abonner",
    // pt [hf]
    "Obrigado por assistir",
    "Obrigada por assistir",
    "Inscreva-se no canal",
    "Se inscreva no canal",
    "Não se esqueça de se inscrever",
    // it [hf]
    "Grazie per la visione",
    "Iscriviti al canale",
    "Non dimenticare di iscriverti",
    // nl [hf]
    "Bedankt voor het kijken",
    // pl [hf]
    "Dziękuję za oglądanie",
    "Dzięki za oglądanie",
    "Subskrybuj mój kanał",
    // ru [hf] [vexa]
    "Спасибо за просмотр",
    "Подписывайтесь на канал",
    "Подписывайтесь на мой канал",
    "Подписывайтесь на наш канал",
    "Не забудьте подписаться",
    "Ставьте лайки и подписывайтесь",
    "Продолжение следует",
    "Субтитры подогнал «Симон»",
    "Спасибо за субтитры Алексею Дубровскому",
    // uk [hf]
    "Дякую за перегляд",
    "Підписуйтесь на наш канал",
    // ja [hf] [vexa] [st]
    "ご視聴ありがとうございました",
    "ご視聴ありがとうございます",
    "最後までご視聴いただきありがとうございました",
    "見てくれてありがとう",
    "チャンネル登録お願いします",
    "チャンネル登録をお願いします",
    "チャンネル登録よろしくお願いします",
    "チャンネル登録をよろしくお願いします",
    "次の動画でお会いしましょう",
    // zh [hf]
    "谢谢观看",
    "謝謝觀看",
    "感谢观看",
    // ko [hf] [st] [tacit]
    "시청해주셔서 감사합니다",
    "시청해 주셔서 감사합니다",
    "오늘도 시청해주셔서 감사합니다",
    "구독과 좋아요 부탁드려요",
    "구독과 좋아요 부탁드립니다",
    "자막 제공 및 광고를 포함하고 있습니다",
    // ar [hf] [st]
    "شكرا على المشاهدة",
    "شكرا للمشاهدة",
    "اشتركوا في القناة",
    // id [hf] [st]
    "Terima kasih telah menonton",
    "Terima kasih sudah menonton",
];

/// Subtitle-credit patterns, matched anywhere in the [`normalize`]d text.
///
/// Each pattern is passed through [`fold`] before compiling, so it may be written with
/// accents or in composed Hangul/kana like the input. Use only lowercase escapes
/// (`\b`, `\s`, `\d`, `\w`): folding lowercases the pattern.
const CREDIT_PATTERNS: &[&str] = &[
    r"\baltyazi\s+m\s*k\b",
    r"\baltyazilar?\b.*\btarafindan\b",
    // Any language [928].
    r"\bamara\s+org\b",
    // de [928]
    r"\buntertitel(?:ung)?\b.*\b(?:zdf|stephanie geiges)\b",
    r"^(?:copyright wdr|swr) \d{4}$",
    // fr [928]
    r"^sous titres? (?:réalisés|faits?) par\b",
    r"\bsous titrage st ?501\b",
    r"\bsoustitreur com\b",
    // it [928]
    r"\bsottotitoli\b.*\bqtss\b",
    // nl [hf] [st]
    r"^(?:c )?tv gelderland \d{4}$",
    // ru [vexa] [928] [st]
    r"\bdimatorzok\b",
    r"\bредактор субтитров\b",
    // zh [hf] [stt46]
    r"明[镜鏡][与與][点點]{2}|打[赏賞]支持明[镜鏡]",
    // ko [st] [tacit]
    r"^mbc 뉴스 \w+입니다$",
    // ar [2608]
    r"نانسي قنقر",
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
        .map(|p| Regex::new(&fold(p)).expect("valid regex literal"))
        .collect()
});

#[expect(
    clippy::expect_used,
    reason = "static regex built from a literal; invalid literal is a programmer error"
)]
static UNSPACED_CHAR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[\p{Han}\p{Hiragana}\p{Katakana}\p{Thai}\p{Lao}\p{Khmer}\p{Myanmar}]")
        .expect("valid regex literal")
});

/// Word count of a normalized key; characters of unspaced scripts count one each.
fn word_count(key: &str) -> usize {
    key.split_whitespace()
        .map(|w| UNSPACED_CHAR.find_iter(w).count().max(1))
        .sum()
}

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
        && word_count(&normalize(&seg.text)) <= SHORT_SEGMENT_MAX_WORDS
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
    fn multilingual_hallucinations_are_detected() {
        for t in [
            " ¡Gracias por ver el vídeo!",
            " Untertitelung des ZDF für funk, 2017.",
            " Copyright WDR 2021",
            " Merci d'avoir regardé cette vidéo !",
            " — Sous-titrage ST' 501 —",
            " Obrigado por assistir.",
            " Sottotitoli e revisione a cura di QTSS",
            " Bedankt voor het kijken.",
            " Dziękuję za oglądanie. Subskrybuj mój kanał.",
            " Продолжение следует...",
            " Субтитры создавал DimaTorzok",
            " Дякую за перегляд!",
            " ご視聴ありがとうございました。",
            " 请不吝点赞 订阅 转发 打赏支持明镜与点点栏目",
            " 謝謝觀看。謝謝觀看。",
            " 시청해주셔서 감사합니다!",
            " MBC 뉴스 이덕영입니다.",
            " ترجمة نانسي قنقر",
            " Terima kasih sudah menonton.",
        ] {
            assert!(is_known_hallucination(t), "should drop {t:?}");
        }
    }

    #[test]
    fn multilingual_real_sentences_are_kept() {
        for t in [
            " Gracias.",
            " Gracias por ver el video que te mandé ayer.",
            " Die Untertitel sind fertig.",
            " Les sous-titres sont prêts pour demain.",
            " Obrigado por assistir à reunião de ontem.",
            " Спасибо за просмотр документа.",
            " 감사합니다.",
            " ご視聴ありがとうございましたと言って配信を終えた。",
            " 谢谢观看我们的产品演示，请提出意见。",
            " شكرا",
        ] {
            assert!(!is_known_hallucination(t), "should keep {t:?}");
        }
    }

    #[test]
    fn unspaced_sentence_is_not_a_short_segment() {
        let f = filter_segments(vec![seg(" 我们明天上午开会讨论预算", 0.7, -0.1, 0.0, 2.0)]);
        assert_eq!(f.dropped, 0);
        let short = filter_segments(vec![seg(" 好的", 0.7, -0.1, 0.0, 1.0)]);
        assert_eq!(short.dropped, 1);
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
