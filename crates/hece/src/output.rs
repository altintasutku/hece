//! The only module that writes to stdout/stderr.
#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "user-facing CLI output"
)]

use std::io::Write;

use hece_core::metrics::{LongReport, ShortReport};

fn pct(v: f64) -> String {
    format!("{:.1}%", v * 100.0)
}

/// One table row; columns line up under [`header_line`]. A set with no scored file shows
/// `n/a` for the averaged columns.
pub fn format_row(set: &str, r: &ShortReport) -> String {
    let na = || "n/a".to_owned();
    let scored = r.files > 0;
    let (wer, cer, names, utt) = if scored {
        (
            pct(r.wer),
            pct(r.cer),
            format!("{}/{}", r.names_hit, r.names_total),
            format!("{:.2}", r.seconds_per_utterance),
        )
    } else {
        (na(), na(), na(), na())
    };
    format!(
        "{set:<6}{:>5}{:>8}{wer:>9}{cer:>9}{names:>8}{:>10}{utt:>8}",
        r.files, r.failed, r.invented,
    )
}

/// Header aligned with [`format_row`].
pub fn header_line() -> String {
    format!(
        "{:<6}{:>5}{:>8}{:>9}{:>9}{:>8}{:>10}{:>8}",
        "set", "files", "failed", "WER", "CER", "names", "invented", "s/utt"
    )
}

/// Extra lines of the long set; `short_wer` enables the delta line.
pub fn format_long_lines(r: &LongReport, short_wer: Option<f64>) -> Vec<String> {
    // No delta when either set scored nothing: a mean over zero files is not a WER.
    let delta = short_wer.filter(|_| r.short.files > 0).map_or_else(
        || "n/a".to_owned(),
        |s| format!("{:+.1}%", (r.short.wer - s) * 100.0),
    );
    vec![
        format!("deleted_runs_ge5: {}", r.deleted_runs),
        format!("repeated_sentences: {}", r.repeated_sentences),
        format!("repeated_5grams: {}", r.repeated_ngrams),
        format!("dropped_negations: {}", r.dropped_negations),
        format!("wer_delta_vs_short: {delta}"),
    ]
}

/// Prints a line to stdout. A closed pipe (e.g. `| head`) ends the process cleanly.
pub fn line(text: &str) {
    let mut out = std::io::stdout().lock();
    if let Err(e) = writeln!(out, "{text}") {
        std::process::exit(i32::from(e.kind() != std::io::ErrorKind::BrokenPipe));
    }
}

/// Prints a progress line to stderr, keeping stdout for results.
pub fn status(text: &str) {
    eprintln!("{text}");
}

/// Prints an error line to stderr.
pub fn error(text: &str) {
    eprintln!("error: {text}");
}

/// Reports one failed file: its name and a stable code, never any text.
pub fn failure(file: &str, code: &str) {
    eprintln!("failed {file} {code}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use hece_core::metrics::ShortReport;

    #[test]
    fn empty_set_shows_na() {
        let r = ShortReport {
            files: 0,
            failed: 3,
            wer: 0.0,
            cer: 0.0,
            names_hit: 0,
            names_total: 0,
            invented: 0,
            seconds_per_utterance: 0.0,
        };
        assert_eq!(
            format_row("long", &r),
            "long      0       3      n/a      n/a     n/a         0     n/a"
        );
    }

    #[test]
    fn short_row_is_stable() {
        let r = ShortReport {
            files: 12,
            failed: 1,
            wer: 0.1214,
            cer: 0.05,
            names_hit: 3,
            names_total: 4,
            invented: 2,
            seconds_per_utterance: 0.456,
        };
        assert_eq!(
            format_row("short", &r),
            "short    12       1    12.1%     5.0%     3/4         2    0.46"
        );
    }
}
