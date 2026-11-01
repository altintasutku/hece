//! Low-confidence signal from duration-weighted `avg_logprob`. Never drops text.

use crate::types::Segment;

/// Weighted mean `avg_logprob` below this marks a transcript as low confidence.
pub const LOW_CONFIDENCE_LOGPROB: f32 = -0.45;

/// Whether the duration-weighted mean `avg_logprob` of the kept segments is below
/// [`LOW_CONFIDENCE_LOGPROB`]. A signal only; it never drops text.
#[must_use]
pub fn is_low_confidence(kept: &[Segment]) -> bool {
    if kept.is_empty() {
        return false;
    }
    let (sum, weights) = kept.iter().fold((0.0_f64, 0.0_f64), |(sum, weights), s| {
        let w = f64::from(s.duration_s().max(0.01));
        (f64::from(s.avg_logprob).mul_add(w, sum), weights + w)
    });
    sum / weights < f64::from(LOW_CONFIDENCE_LOGPROB)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(logprob: f32, start: f32, end: f32) -> Segment {
        Segment {
            text: " x".to_owned(),
            start_s: start,
            end_s: end,
            no_speech_prob: 0.0,
            avg_logprob: logprob,
        }
    }

    #[test]
    fn empty_is_not_low() {
        assert!(!is_low_confidence(&[]));
    }

    #[test]
    fn weighted_mean_below_threshold_is_low() {
        assert!(is_low_confidence(&[
            seg(-0.2, 0.0, 2.0),
            seg(-1.5, 2.0, 2.5)
        ]));
    }

    #[test]
    fn weighted_mean_above_threshold_is_not_low() {
        assert!(!is_low_confidence(&[
            seg(-0.2, 0.0, 2.0),
            seg(-1.4, 2.0, 2.5)
        ]));
    }

    #[test]
    fn zero_duration_weighs_ten_ms() {
        assert!(is_low_confidence(&[seg(-0.9, 1.0, 1.0)]));
        // 2 s at -0.1 dominates a zero-duration -5.0 segment (weight 0.01)
        assert!(!is_low_confidence(&[
            seg(-0.1, 0.0, 2.0),
            seg(-5.0, 2.0, 2.0)
        ]));
    }
}
