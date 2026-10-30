//! Splitting one window's tokens into timed segments.
//!
//! Ported from faster-whisper 1.2.1 (MIT), `transcribe.py`
//! (`WhisperModel._split_segments_by_timestamps`).

/// Mel frames per output token position (`WhisperModel.input_stride`).
pub const INPUT_STRIDE: usize = 2;
/// Seconds per timestamp token step (`WhisperModel.time_precision`).
pub const TIME_PRECISION: f64 = 0.02;

/// A segment before text decoding: times in seconds and its tokens (timestamps included).
#[derive(Debug, Clone, PartialEq)]
pub struct RawSegment {
    /// Start time in seconds.
    pub start: f64,
    /// End time in seconds.
    pub end: f64,
    /// Tokens of the segment, including its timestamp tokens.
    pub tokens: Vec<u32>,
}

/// Splits `tokens` at consecutive timestamp pairs. Returns the segments, the next seek (in mel
/// frames) and whether the window ended on a single timestamp.
#[must_use]
pub fn split_by_timestamps(
    timestamp_begin: u32,
    tokens: &[u32],
    time_offset: f64,
    segment_size: usize,
    segment_duration: f64,
    seek: usize,
) -> (Vec<RawSegment>, usize, bool) {
    let is_ts = |t: u32| t >= timestamp_begin;
    // Signed so a slice that starts with a text token gives the same (negative) offset as Python.
    let position = |t: u32| i64::from(t) - i64::from(timestamp_begin);
    let time_at = |t: u32| time_offset + position_secs(position(t));

    let n = tokens.len();
    let single_timestamp_ending = n >= 2 && !is_ts(tokens[n - 2]) && is_ts(tokens[n - 1]);
    let consecutive: Vec<usize> = (1..n)
        .filter(|&i| is_ts(tokens[i]) && is_ts(tokens[i - 1]))
        .collect();

    let mut segments = Vec::new();
    let mut seek = seek;
    if consecutive.is_empty() {
        let mut duration = segment_duration;
        if let Some(&last) = tokens.iter().rev().find(|&&t| is_ts(t))
            && last != timestamp_begin
        {
            duration = position_secs(position(last));
        }
        segments.push(RawSegment {
            start: time_offset,
            end: time_offset + duration,
            tokens: tokens.to_vec(),
        });
        seek += segment_size;
    } else {
        let mut slices = consecutive;
        if single_timestamp_ending {
            slices.push(n);
        }
        let mut last_slice = 0;
        for current in slices {
            let part = &tokens[last_slice..current];
            // Slices are non-empty: they end at increasing indices greater than 0.
            if let (Some(&first), Some(&last)) = (part.first(), part.last()) {
                segments.push(RawSegment {
                    start: time_at(first),
                    end: time_at(last),
                    tokens: part.to_vec(),
                });
            }
            last_slice = current;
        }
        if single_timestamp_ending {
            // A single timestamp at the end means no speech after it.
            seek += segment_size;
        } else {
            // Drop the unfinished segment and seek to the last timestamp. `tokens[last_slice - 1]`
            // is the second token of a consecutive pair, so it is a timestamp.
            let last_position = tokens[last_slice - 1].saturating_sub(timestamp_begin);
            seek += last_position as usize * INPUT_STRIDE;
        }
    }
    (segments, seek, single_timestamp_ending)
}

/// Seconds for a timestamp position.
#[expect(
    clippy::cast_precision_loss,
    reason = "timestamp positions are below 1501, exact in f64"
)]
fn position_secs(position: i64) -> f64 {
    position as f64 * TIME_PRECISION
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn ids(v: &Value) -> Vec<u32> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|x| u32::try_from(x.as_u64().unwrap()).unwrap())
            .collect()
    }

    fn usize_of(v: &Value) -> usize {
        usize::try_from(v.as_u64().unwrap()).unwrap()
    }

    #[test]
    fn matches_faster_whisper_fixture() {
        let fixture: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/split_expected.json")).unwrap();
        let tb = u32::try_from(fixture["timestamp_begin"].as_u64().unwrap()).unwrap();
        let cases = fixture["cases"].as_array().unwrap();
        assert!(cases.len() >= 6);
        for case in cases {
            let name = case["name"].as_str().unwrap();
            let (segments, next_seek, single) = split_by_timestamps(
                tb,
                &ids(&case["tokens"]),
                case["time_offset"].as_f64().unwrap(),
                usize_of(&case["segment_size"]),
                case["segment_duration"].as_f64().unwrap(),
                usize_of(&case["seek"]),
            );
            let expected = case["segments"].as_array().unwrap();
            assert_eq!(segments.len(), expected.len(), "{name}: segment count");
            for (got, want) in segments.iter().zip(expected) {
                assert!(
                    (got.start - want["start"].as_f64().unwrap()).abs() < 1e-4,
                    "{name}"
                );
                assert!(
                    (got.end - want["end"].as_f64().unwrap()).abs() < 1e-4,
                    "{name}"
                );
                assert_eq!(got.tokens, ids(&want["tokens"]), "{name}: tokens");
            }
            assert_eq!(next_seek, usize_of(&case["next_seek"]), "{name}: seek");
            assert_eq!(
                single,
                case["single_timestamp_ending"].as_bool().unwrap(),
                "{name}: single ending"
            );
        }
    }

    #[test]
    fn empty_tokens_give_one_window_long_segment() {
        let (segments, seek, single) = split_by_timestamps(50_365, &[], 0.0, 3000, 30.0, 0);
        assert_eq!(segments.len(), 1);
        assert!(segments[0].tokens.is_empty());
        assert_eq!((seek, single), (3000, false));
    }
}
