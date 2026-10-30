//! The only module that writes to stdout/stderr.
#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "user-facing CLI output"
)]

use hece_core::metrics::{LongReport, ShortReport};

fn pct(v: f64) -> String {
    format!("{:.1}%", v * 100.0)
}

/// One table row; columns line up under [`header_line`].
pub fn format_row(set: &str, r: &ShortReport) -> String {
    format!(
        "{set:<6}{:>5}{:>8}{:>9}{:>9}{:>8}{:>10}{:>8.2}",
        r.files,
        r.failed,
        pct(r.wer),
        pct(r.cer),
        format!("{}/{}", r.names_hit, r.names_total),
        r.invented,
        r.seconds_per_utterance,
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
    let delta = short_wer.map_or_else(
        || "n/a".to_owned(),
        |s| format!("{:+.1}%", (r.short.wer - s) * 100.0),
    );
    vec![
        format!("deleted_runs_ge5: {}", r.deleted_runs),
        format!("repeated_sentences: {}", r.repeated_sentences),
        format!("dropped_negations: {}", r.dropped_negations),
        format!("wer_delta_vs_short: {delta}"),
    ]
}

/// Prints a line to stdout.
pub fn line(text: &str) {
    println!("{text}");
}

/// Prints an error line to stderr.
pub fn error(text: &str) {
    eprintln!("error: {text}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use hece_core::metrics::ShortReport;

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
