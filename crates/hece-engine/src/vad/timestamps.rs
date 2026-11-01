//! Speech-timestamp state machine, chunk collection and timestamp restoration.
//!
//! Ported from faster-whisper 1.2.1 (MIT), `vad.py` (`get_speech_timestamps`,
//! `collect_chunks`, `SpeechTimestampsMap`). Sample math is done in `f64`/`i64` like the Python
//! (which mixes Python floats and ints) so boundaries match exactly.

use super::WINDOW;
use crate::audio::SAMPLE_RATE;

/// Options of the speech-timestamp state machine; defaults are faster-whisper's.
#[derive(Debug, Clone, PartialEq)]
pub struct VadOptions {
    /// Probabilities at or above this are speech.
    pub threshold: f32,
    /// Probabilities below this end speech; `None` means `max(threshold - 0.15, 0.01)`.
    pub neg_threshold: Option<f32>,
    /// Speech chunks not longer than this are dropped.
    pub min_speech_duration_ms: u32,
    /// Chunks longer than this are split (at a pause if one exists).
    pub max_speech_duration_s: f32,
    /// Silence needed to close a speech chunk.
    pub min_silence_duration_ms: u32,
    /// Padding added on each side of a chunk.
    pub speech_pad_ms: u32,
}

impl Default for VadOptions {
    fn default() -> Self {
        Self {
            threshold: 0.5,
            neg_threshold: None,
            min_speech_duration_ms: 0,
            max_speech_duration_s: f32::INFINITY,
            min_silence_duration_ms: 2000,
            speech_pad_ms: 400,
        }
    }
}

/// A speech region as a half-open sample range `[start, end)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpeechSpan {
    /// First sample.
    pub start: usize,
    /// One past the last sample.
    pub end: usize,
}

const SR: f64 = 16_000.0;
const WINDOW_F: f64 = 512.0;
const _: () = assert!(SAMPLE_RATE == 16_000 && WINDOW == 512);

#[expect(
    clippy::cast_precision_loss,
    reason = "sample counts are far below 2^52"
)]
fn to_f64(x: i64) -> f64 {
    x as f64
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "Python int() truncation of a sample position"
)]
fn trunc(x: f64) -> i64 {
    x as i64
}

/// Window start sample of probability index `i`.
fn pos(i: usize) -> i64 {
    i64::try_from(i * WINDOW).unwrap_or(i64::MAX)
}

/// Runs the faster-whisper state machine over per-window speech probabilities.
///
/// `probs` has one entry per 512-sample window of the (zero padded) audio; `audio_len` is the
/// unpadded length in samples.
#[must_use]
pub fn speech_timestamps(probs: &[f32], audio_len: usize, opts: &VadOptions) -> Vec<SpeechSpan> {
    // Thresholds are compared in f32 like numpy compares a float32 probability with a Python
    // float; the default negative threshold is derived in f64 first.
    let threshold = opts.threshold;
    #[expect(
        clippy::cast_possible_truncation,
        reason = "narrowing an f64 threshold to the f32 probability domain"
    )]
    let neg_threshold = opts
        .neg_threshold
        .unwrap_or_else(|| (f64::from(threshold) - 0.15).max(0.01) as f32);

    let min_speech_samples = SR * f64::from(opts.min_speech_duration_ms) / 1000.0;
    let speech_pad_samples = SR * f64::from(opts.speech_pad_ms) / 1000.0;
    let max_speech_samples =
        SR * f64::from(opts.max_speech_duration_s) - WINDOW_F - 2.0 * speech_pad_samples;
    let min_silence_samples = SR * f64::from(opts.min_silence_duration_ms) / 1000.0;
    let min_silence_samples_at_max_speech = SR * 98.0 / 1000.0;
    let audio_len = i64::try_from(audio_len).unwrap_or(i64::MAX);

    let mut triggered = false;
    let mut speeches: Vec<(i64, i64)> = Vec::new();
    // Start of the speech currently being built (Python: `current_speech["start"]`).
    let mut current_start: Option<i64> = None;
    let mut temp_end = 0_i64;
    let (mut prev_end, mut next_start) = (0_i64, 0_i64);

    for (i, &prob) in probs.iter().enumerate() {
        let p = pos(i);
        if prob >= threshold && temp_end != 0 {
            temp_end = 0;
            if next_start < prev_end {
                next_start = p;
            }
        }

        if prob >= threshold && !triggered {
            triggered = true;
            current_start = Some(p);
            continue;
        }

        if let (true, Some(start)) = (triggered, current_start)
            && to_f64(p - start) > max_speech_samples
        {
            if prev_end != 0 {
                speeches.push((start, prev_end));
                current_start = None;
                // Silence reached earlier (< neg_threshold) and still not speech (< threshold).
                if next_start < prev_end {
                    triggered = false;
                } else {
                    current_start = Some(next_start);
                }
                prev_end = 0;
                next_start = 0;
                temp_end = 0;
            } else {
                speeches.push((start, p));
                current_start = None;
                prev_end = 0;
                next_start = 0;
                temp_end = 0;
                triggered = false;
                continue;
            }
        }

        if prob < neg_threshold && triggered {
            if temp_end == 0 {
                temp_end = p;
            }
            // Avoid cutting in very short silence.
            if to_f64(p - temp_end) > min_silence_samples_at_max_speech {
                prev_end = temp_end;
            }
            if (to_f64(p - temp_end)) < min_silence_samples {
                continue;
            }
            if let Some(start) = current_start
                && to_f64(temp_end - start) > min_speech_samples
            {
                speeches.push((start, temp_end));
            }
            current_start = None;
            prev_end = 0;
            next_start = 0;
            temp_end = 0;
            triggered = false;
        }
    }

    if let Some(start) = current_start
        && to_f64(audio_len - start) > min_speech_samples
    {
        speeches.push((start, audio_len));
    }

    pad_speeches(&mut speeches, audio_len, speech_pad_samples);

    speeches
        .into_iter()
        .map(|(start, end)| SpeechSpan {
            start: usize::try_from(start).unwrap_or(0),
            end: usize::try_from(end).unwrap_or(0),
        })
        .collect()
}

/// Pads chunk edges; neighbours closer than twice the padding split the gap evenly.
fn pad_speeches(speeches: &mut [(i64, i64)], audio_len: i64, pad: f64) {
    let n = speeches.len();
    for i in 0..n {
        if i == 0 {
            speeches[i].0 = trunc((to_f64(speeches[i].0) - pad).max(0.0));
        }
        if i + 1 == n {
            speeches[i].1 = trunc((to_f64(speeches[i].1) + pad).min(to_f64(audio_len)));
        } else {
            let silence = speeches[i + 1].0 - speeches[i].1;
            if to_f64(silence) < 2.0 * pad {
                // Python floor division.
                let half = silence.div_euclid(2);
                speeches[i].1 += half;
                speeches[i + 1].0 = (speeches[i + 1].0 - half).max(0);
            } else {
                speeches[i].1 = trunc((to_f64(speeches[i].1) + pad).min(to_f64(audio_len)));
                speeches[i + 1].0 = trunc((to_f64(speeches[i + 1].0) - pad).max(0.0));
            }
        }
    }
}

/// Concatenates the speech spans of `audio` into one buffer (faster-whisper's `collect_chunks`
/// with an unlimited `max_duration`). No spans yield an empty buffer.
#[must_use]
pub fn collect_chunks(audio: &[f32], spans: &[SpeechSpan]) -> Vec<f32> {
    let mut out = Vec::new();
    for span in spans {
        let end = span.end.min(audio.len());
        let start = span.start.min(end);
        out.extend_from_slice(&audio[start..end]);
    }
    out
}

/// Maps times in the collected (speech-only) audio back to times in the original audio.
#[derive(Debug, Clone)]
pub struct SpeechTimestampsMap {
    chunk_end_sample: Vec<i64>,
    total_silence_before: Vec<f64>,
}

impl SpeechTimestampsMap {
    /// Builds the map from the spans that were collected.
    #[must_use]
    pub fn new(spans: &[SpeechSpan]) -> Self {
        let mut chunk_end_sample = Vec::with_capacity(spans.len());
        let mut total_silence_before = Vec::with_capacity(spans.len());
        let mut previous_end = 0_i64;
        let mut silent_samples = 0_i64;
        for span in spans {
            let start = i64::try_from(span.start).unwrap_or(i64::MAX);
            let end = i64::try_from(span.end).unwrap_or(i64::MAX);
            silent_samples += start - previous_end;
            previous_end = end;
            chunk_end_sample.push(end - silent_samples);
            total_silence_before.push(to_f64(silent_samples) / SR);
        }
        Self {
            chunk_end_sample,
            total_silence_before,
        }
    }

    /// Original-audio time for `t` seconds into the collected audio, rounded to 2 decimals like
    /// faster-whisper. `is_end` makes a time exactly on a chunk end belong to that chunk. All
    /// math is `f64` as in Python so boundary comparisons agree.
    #[must_use]
    pub fn original_time(&self, t: f64, is_end: bool) -> f64 {
        let offset = self
            .chunk_index(t, is_end)
            .map_or(0.0, |i| self.total_silence_before[i]);
        round2(offset + t)
    }

    fn chunk_index(&self, t: f64, is_end: bool) -> Option<usize> {
        let last = self.chunk_end_sample.len().checked_sub(1)?;
        let sample = trunc(t * SR);
        if is_end && let Some(i) = self.chunk_end_sample.iter().position(|&e| e == sample) {
            return Some(i);
        }
        // bisect_right
        let i = self.chunk_end_sample.partition_point(|&e| e <= sample);
        Some(i.min(last))
    }
}

fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}
