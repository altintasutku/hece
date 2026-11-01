//! Turkish text normalization for benchmarks: number and acronym expansion, Turkish
//! lowercase and punctuation stripping.
//!
//! Both sides of a comparison (reference and hypothesis) are brought to the same form:
//! numbers are spelled out, upper-case acronyms are read letter by letter, text is
//! lowercased the Turkish way, punctuation is dropped and an apostrophe-separated suffix
//! is glued to its word (`yedi'de` becomes `yedide`).
//!
//! The regexes need lookaround, so they use `fancy-regex`. Its matcher can fail at runtime
//! (backtrack limit); this module is pure and has no logger, so a failed substitution
//! leaves the text of that step unchanged.

use std::{fmt::Write as _, sync::LazyLock};

use fancy_regex::{Captures, Regex, Replacer};
use regex::Regex as PlainRegex;
use unicode_normalization::UnicodeNormalization;

const ONES: [&str; 10] = [
    "", "bir", "iki", "üç", "dört", "beş", "altı", "yedi", "sekiz", "dokuz",
];
const TENS: [&str; 10] = [
    "", "on", "yirmi", "otuz", "kırk", "elli", "altmış", "yetmiş", "seksen", "doksan",
];
/// Largest to smallest. "bir bin" is never said ("bin"), "bir milyon" is.
const SCALES: [(u64, &str); 3] = [
    (1_000_000_000, "milyar"),
    (1_000_000, "milyon"),
    (1_000, "bin"),
];
/// Largest number [`number_to_words`] spells out.
pub const MAX_NUMBER: u64 = 999_999_999_999;

const MONTHS: [&str; 12] = [
    "Ocak", "Şubat", "Mart", "Nisan", "Mayıs", "Haziran", "Temmuz", "Ağustos", "Eylül", "Ekim",
    "Kasım", "Aralık",
];

/// Spelled-out letter names. K and H are "ka" and "ha" as in everyday use (KVKK, THY).
const LETTER_NAMES: [(char, &str); 29] = [
    ('A', "a"),
    ('B', "be"),
    ('C', "ce"),
    ('Ç', "çe"),
    ('D', "de"),
    ('E', "e"),
    ('F', "fe"),
    ('G', "ge"),
    ('Ğ', "yumuşak ge"),
    ('H', "ha"),
    ('I', "ı"),
    ('İ', "i"),
    ('J', "je"),
    ('K', "ka"),
    ('L', "le"),
    ('M', "me"),
    ('N', "ne"),
    ('O', "o"),
    ('Ö', "ö"),
    ('P', "pe"),
    ('Q', "kü"),
    ('R', "re"),
    ('S', "se"),
    ('Ş', "şe"),
    ('T', "te"),
    ('U', "u"),
    ('Ü', "ü"),
    ('V', "ve"),
    ('W', "dabılyu"),
];
const LETTER_NAMES_TAIL: [(char, &str); 3] = [('X', "iks"), ('Y', "ye"), ('Z', "ze")];
const VOWELS: &str = "AEIİOÖUÜ";

// A digit must not be part of a word ("mp3", "B2"), so word-char neighbours are excluded.
const NOT_AFTER: &str = r"(?<![\w.,:])";
const NOT_BEFORE: &str = r"(?![\w]|[.,:]\d)";
// An apostrophe after a number is dropped with the match: "14:00'te" -> "on dörtte".
const SUFFIX: &str = r"(?:['’](?=\w))?";
const AMOUNT: &str = r"(\d{1,3}(?:\.\d{3})+|\d+)(?:,(\d{1,2}))?";

#[expect(
    clippy::expect_used,
    reason = "static regex built from a literal; an invalid literal is a programmer error"
)]
fn fancy(pattern: &str) -> Regex {
    Regex::new(pattern).expect("valid regex literal")
}

static DATE: LazyLock<Regex> = LazyLock::new(|| {
    fancy(&format!(
        r"{NOT_AFTER}(\d{{1,2}})[./](\d{{1,2}})[./](\d{{4}}){NOT_BEFORE}{SUFFIX}"
    ))
});
static TIME: LazyLock<Regex> = LazyLock::new(|| {
    fancy(&format!(
        r"{NOT_AFTER}([01]?\d|2[0-3])[:.]([0-5]\d){NOT_BEFORE}{SUFFIX}"
    ))
});
static PERCENT: LazyLock<Regex> =
    LazyLock::new(|| fancy(&format!(r"%\s?(\d+(?:,\d+)?){NOT_BEFORE}{SUFFIX}")));
static DECIMAL: LazyLock<Regex> =
    LazyLock::new(|| fancy(&format!(r"{NOT_AFTER}(\d+),(\d+){NOT_BEFORE}{SUFFIX}")));
static MONEY: LazyLock<Regex> = LazyLock::new(|| {
    fancy(&format!(
        r"{NOT_AFTER}(?:₺\s?)?{AMOUNT}\s?(?:₺|TL\b|TRY\b)|₺\s?{AMOUNT}{NOT_BEFORE}{SUFFIX}"
    ))
});
static BARE_CURRENCY: LazyLock<Regex> = LazyLock::new(|| fancy(r"₺|(?<![\w])TL(?![\w])"));
static INTEGER: LazyLock<Regex> = LazyLock::new(|| {
    fancy(&format!(
        r"{NOT_AFTER}(\d{{1,3}}(?:\.\d{{3}})+|\d+){NOT_BEFORE}{SUFFIX}"
    ))
});
static ACRONYM: LazyLock<Regex> = LazyLock::new(|| fancy(r"(?<![\w'’])([A-ZÇĞİÖŞÜ]{2,6})(?![\w])"));
static ALNUM_ACRONYM: LazyLock<Regex> =
    LazyLock::new(|| fancy(r"(?<![\w'’])([A-ZÇĞİÖŞÜ]{1,6})(\d{1,4})(?![\w])"));
static APOSTROPHE: LazyLock<Regex> = LazyLock::new(|| fancy(r"(?<=\w)['’](?=\w)"));
#[expect(
    clippy::expect_used,
    reason = "static regex built from a literal; an invalid literal is a programmer error"
)]
static NON_WORD: LazyLock<PlainRegex> =
    LazyLock::new(|| PlainRegex::new(r"[^\w\s]|_").expect("valid regex literal"));

/// Replaces every match; a matcher error (backtrack limit) leaves `text` unchanged.
fn substitute<R: Replacer>(re: &Regex, text: &str, rep: R) -> String {
    re.try_replacen(text, 0, rep)
        .map_or_else(|_| text.to_owned(), std::borrow::Cow::into_owned)
}

fn group<'t>(caps: &Captures<'t>, index: usize) -> Option<&'t str> {
    caps.get(index).map(|m| m.as_str())
}

fn whole_match<'t>(caps: &Captures<'t>) -> &'t str {
    group(caps, 0).unwrap_or_default()
}

/// Parses a number written with `.` thousands separators; `None` if it exceeds [`MAX_NUMBER`]
/// (or is not parseable, e.g. non-ASCII digits).
fn parse_amount(text: &str) -> Option<u64> {
    let value = text.replace('.', "").parse::<u64>().ok()?;
    (value <= MAX_NUMBER).then_some(value)
}

/// Spells a number in Turkish ("1250" becomes "bin iki yüz elli", 0 becomes "sıfır").
///
/// Supports `0..=999_999_999_999`; larger values are returned as decimal digits.
#[must_use]
pub fn number_to_words(n: u64) -> String {
    if n > MAX_NUMBER {
        return n.to_string();
    }
    if n == 0 {
        return "sıfır".to_owned();
    }
    let mut words: Vec<String> = Vec::new();
    let mut rest = n;
    for (value, name) in SCALES {
        let count = rest / value;
        rest %= value;
        if count == 0 {
            continue;
        }
        if !(count == 1 && value == 1_000) {
            words.push(below_thousand(count));
        }
        words.push(name.to_owned());
    }
    if rest > 0 {
        words.push(below_thousand(rest));
    }
    words.join(" ")
}

/// Looks a word up by index; callers keep the index in range.
fn table_word(table: &[&'static str], index: u64) -> &'static str {
    usize::try_from(index)
        .ok()
        .and_then(|i| table.get(i))
        .copied()
        .unwrap_or_default()
}

fn below_thousand(n: u64) -> String {
    let (hundreds, rest) = (n / 100, n % 100);
    let (tens, ones) = (rest / 10, rest % 10);
    let mut words: Vec<&str> = Vec::new();
    if hundreds > 0 {
        if hundreds > 1 {
            words.push(table_word(&ONES, hundreds));
        }
        words.push("yüz");
    }
    if tens > 0 {
        words.push(table_word(&TENS, tens));
    }
    if ones > 0 {
        words.push(table_word(&ONES, ones));
    }
    words.join(" ")
}

/// Fraction digits: leading zeros are read one by one ("05" becomes "sıfır beş").
/// `None` if the remainder is out of range.
fn digits_to_words(digits: &str) -> Option<String> {
    let trimmed = digits.trim_start_matches('0');
    let mut words = vec!["sıfır".to_owned(); digits.len() - trimmed.len()];
    if !trimmed.is_empty() {
        words.push(number_to_words(parse_amount(trimmed)?));
    }
    Some(words.join(" "))
}

fn decimal_words(whole: &str, fraction: &str) -> Option<String> {
    Some(format!(
        "{} virgül {}",
        number_to_words(parse_amount(whole)?),
        digits_to_words(fraction)?
    ))
}

fn replace_integer(caps: &Captures<'_>) -> String {
    group(caps, 1)
        .and_then(parse_amount)
        .map_or_else(|| whole_match(caps).to_owned(), number_to_words)
}

fn replace_date(caps: &Captures<'_>) -> String {
    let parse = |i| group(caps, i).and_then(|s| s.parse::<u64>().ok());
    if let (Some(day), Some(month), Some(year)) = (parse(1), parse(2), parse(3))
        && (1..=31).contains(&day)
        && (1..=12).contains(&month)
    {
        return format!(
            "{} {} {}",
            number_to_words(day),
            table_word(&MONTHS, month - 1),
            number_to_words(year)
        );
    }
    whole_match(caps).to_owned()
}

fn replace_time(caps: &Captures<'_>) -> String {
    let parse = |i| group(caps, i).and_then(|s| s.parse::<u64>().ok());
    match (parse(1), parse(2)) {
        (Some(hour), Some(0)) => number_to_words(hour),
        (Some(hour), Some(minute)) => {
            format!("{} {}", number_to_words(hour), number_to_words(minute))
        }
        _ => whole_match(caps).to_owned(),
    }
}

fn replace_percent(caps: &Captures<'_>) -> String {
    let spoken = group(caps, 1).and_then(|number| match number.split_once(',') {
        Some((whole, fraction)) => decimal_words(whole, fraction),
        None => parse_amount(number).map(number_to_words),
    });
    spoken.map_or_else(|| whole_match(caps).to_owned(), |s| format!("yüzde {s}"))
}

fn replace_decimal(caps: &Captures<'_>) -> String {
    match (group(caps, 1), group(caps, 2)) {
        (Some(whole), Some(fraction)) => decimal_words(whole, fraction),
        _ => None,
    }
    .unwrap_or_else(|| whole_match(caps).to_owned())
}

fn replace_money(caps: &Captures<'_>) -> String {
    // Two alternatives: groups 1-2 (amount before the symbol), 3-4 (after `₺`).
    let (whole, fraction) = if caps.get(1).is_some() {
        (group(caps, 1), group(caps, 2))
    } else {
        (group(caps, 3), group(caps, 4))
    };
    let Some(value) = whole.and_then(parse_amount) else {
        return whole_match(caps).to_owned();
    };
    // A single fraction digit means tens ("12,5" is 50 kuruş).
    let cents = fraction.map_or(0, |f| format!("{f:0<2}").parse::<u64>().unwrap_or(0));
    if value == 0 && cents > 0 {
        return format!("{} kuruş", number_to_words(cents));
    }
    let spoken = format!("{} lira", number_to_words(value));
    if cents > 0 {
        format!("{spoken} {} kuruş", number_to_words(cents))
    } else {
        spoken
    }
}

/// Spells out money, dates, times, percentages, decimals and integers; order matters (most
/// specific pattern first).
#[must_use]
pub fn expand_numbers(text: &str) -> String {
    let text = substitute(&MONEY, text, |c: &Captures<'_>| replace_money(c));
    let text = substitute(&BARE_CURRENCY, &text, "lira");
    let text = substitute(&DATE, &text, |c: &Captures<'_>| replace_date(c));
    let text = substitute(&TIME, &text, |c: &Captures<'_>| replace_time(c));
    let text = substitute(&PERCENT, &text, |c: &Captures<'_>| replace_percent(c));
    let text = substitute(&DECIMAL, &text, |c: &Captures<'_>| replace_decimal(c));
    substitute(&INTEGER, &text, |c: &Captures<'_>| replace_integer(c))
}

/// Turkish lowercase: `I` becomes `ı` and `İ` becomes `i` (plain lowercasing gets both wrong).
#[must_use]
pub fn turkish_lower(text: &str) -> String {
    text.replace('I', "ı").replace('İ', "i").to_lowercase()
}

fn letter_name(c: char) -> Option<&'static str> {
    LETTER_NAMES
        .iter()
        .chain(LETTER_NAMES_TAIL.iter())
        .find_map(|&(letter, name)| (letter == c).then_some(name))
}

/// An acronym that fits Turkish syllable structure is read as a word (TÜBİTAK, ODTÜ); the rest
/// letter by letter (EEM, ABD, THY). Rough rule: no two adjacent vowels, no consonant cluster
/// at the start or end, at most two consonants in a row.
fn pronounceable(word: &str) -> bool {
    let pattern: String = word
        .chars()
        .map(|c| if VOWELS.contains(c) { 'V' } else { 'C' })
        .collect();
    pattern.contains('V')
        && !pattern.contains("VV")
        && !pattern.starts_with("CC")
        && !pattern.ends_with("CC")
        && !pattern.contains("CCC")
}

fn letters(word: &str) -> String {
    if word.chars().count() > 1 && pronounceable(word) {
        return word.to_owned();
    }
    let mut out = String::new();
    for (i, c) in word.chars().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        match letter_name(c) {
            Some(name) => out.push_str(name),
            None => out.push(c),
        }
    }
    out
}

/// Spells unpronounceable upper-case acronyms letter by letter: "EEM" becomes "e e me" and
/// "ESP32" becomes "e se pe otuz iki". A suffix after an apostrophe is kept.
#[must_use]
pub fn spell_acronyms(text: &str) -> String {
    let text = substitute(&ALNUM_ACRONYM, text, |caps: &Captures<'_>| {
        let digits = group(caps, 2)
            .and_then(|d| d.parse::<u64>().ok())
            .unwrap_or(0);
        let mut out = letters(group(caps, 1).unwrap_or_default());
        // Writing to a `String` cannot fail.
        let _ = write!(out, " {}", number_to_words(digits));
        out
    });
    substitute(&ACRONYM, &text, |caps: &Captures<'_>| {
        letters(group(caps, 1).unwrap_or_default())
    })
}

/// Normalizes text to comparable words: NFC, numbers and acronyms spelled out, apostrophes
/// between word characters removed, Turkish lowercase, punctuation and `_` dropped.
#[must_use]
pub fn normalize_words(text: &str) -> Vec<String> {
    let text: String = text.nfc().collect();
    let text = spell_acronyms(&expand_numbers(&text));
    let text = substitute(&APOSTROPHE, &text, "");
    NON_WORD
        .replace_all(&turkish_lower(&text), " ")
        .split_whitespace()
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_to_words_spot_values() {
        assert_eq!(number_to_words(0), "sıfır");
        assert_eq!(number_to_words(1), "bir");
        assert_eq!(number_to_words(1_000), "bin");
        assert_eq!(number_to_words(2_025), "iki bin yirmi beş");
        assert_eq!(number_to_words(1_000_000), "bir milyon");
        assert_eq!(number_to_words(100), "yüz");
        assert_eq!(
            number_to_words(MAX_NUMBER),
            "dokuz yüz doksan dokuz milyar dokuz yüz doksan dokuz milyon dokuz yüz doksan dokuz bin dokuz yüz doksan dokuz"
        );
        assert_eq!(number_to_words(MAX_NUMBER + 1), "1000000000000");
    }

    #[test]
    fn out_of_range_and_huge_inputs_do_not_panic() {
        assert_eq!(
            expand_numbers("99999999999999999999999 ve 99999999999999,5"),
            "99999999999999999999999 ve 99999999999999,5"
        );
        assert_eq!(expand_numbers("٣ elma"), "٣ elma");
    }
}
