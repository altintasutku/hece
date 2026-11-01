//! Temperature fallback around one decoder call.
//!
//! Ported from faster-whisper 1.2.1 (MIT), `transcribe.py` (`WhisperModel.generate_with_fallback`,
//! `get_compression_ratio`).

use std::io::Write;

use flate2::Compression;
use flate2::write::ZlibEncoder;

use super::{DecodeOptions, Decoder, GenerateRequest, GenerateResult};
use crate::error::EngineError;
use crate::tokens::{SpecialTokens, TextTokenizer};

/// Maximum decoder length, prompt included (`WhisperModel.max_length`).
pub(super) const MAX_LENGTH: usize = 448;

/// `len(utf8) / len(zlib.compress(utf8))`, with zlib's default level 6 like Python.
#[must_use]
#[expect(
    clippy::cast_precision_loss,
    reason = "byte counts of one window's text are far below 2^52"
)]
pub fn compression_ratio(text: &str) -> f64 {
    let bytes = text.as_bytes();
    let mut enc = ZlibEncoder::new(Vec::new(), Compression::new(6));
    // Writing to a `Vec` cannot fail; on the impossible error fall back to the raw length,
    // which gives a ratio of 1 (not repetitive).
    let compressed = match enc.write_all(bytes).and_then(|()| enc.finish()) {
        Ok(out) => out.len(),
        Err(_) => bytes.len().max(1),
    };
    bytes.len() as f64 / compressed as f64
}

/// Text of `tokens` with timestamp and special ids removed (faster-whisper `Tokenizer.decode`).
pub(super) fn decode_text(
    tok: &dyn TextTokenizer,
    sp: &SpecialTokens,
    tokens: &[u32],
) -> Result<String, EngineError> {
    let text: Vec<u32> = tokens.iter().copied().filter(|&t| t < sp.eot).collect();
    tok.decode(&text)
}

/// The chosen result of [`generate_with_fallback`].
#[derive(Debug, Clone)]
pub(super) struct Decoded {
    /// The decoder output.
    pub result: GenerateResult,
    /// Average log probability per token (end token included).
    pub avg_logprob: f64,
    /// Temperature reported to the prompt-reset rule.
    pub temperature: f64,
}

/// `score * len^length_penalty / (len + 1)`: undoes `CTranslate2`'s length normalisation.
#[expect(
    clippy::cast_precision_loss,
    reason = "token counts are at most 448, exact in f64"
)]
fn avg_logprob(result: &GenerateResult, length_penalty: f32) -> f64 {
    let len = result.tokens.len() as f64;
    let cum = f64::from(result.score) * len.powf(f64::from(length_penalty));
    cum / (len + 1.0)
}

/// Decodes one window, retrying at the next temperature while the result is repetitive or
/// improbable. If every temperature fails, picks the most probable result below the
/// compression threshold (else of all) and reports the last temperature tried.
pub(super) fn generate_with_fallback<D: Decoder>(
    dec: &mut D,
    enc: &D::Encoded,
    prompt: &[u32],
    tok: &dyn TextTokenizer,
    sp: &SpecialTokens,
    suppress: &[i32],
    opts: &DecodeOptions,
) -> Result<Decoded, EngineError> {
    let max_initial_timestamp_index = max_initial_timestamp_index(opts.max_initial_timestamp);
    let mut all: Vec<(Decoded, bool)> = Vec::new();
    let mut last_temperature = 0.0;

    for &temperature in &opts.temperatures {
        last_temperature = temperature;
        // ct2's defaults for the options faster-whisper leaves out of each branch.
        let (beam_size, patience, num_hypotheses, sampling_topk, sampling_temperature) =
            if temperature > 0.0 {
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "ct2 takes the temperature as f32"
                )]
                let t = temperature as f32;
                (1, 1.0, opts.best_of, 0, t)
            } else {
                (opts.beam_size, opts.patience, 1, 1, 1.0)
            };
        let req = GenerateRequest {
            prompt,
            beam_size,
            patience,
            num_hypotheses,
            sampling_temperature,
            sampling_topk,
            max_length: MAX_LENGTH,
            max_initial_timestamp_index,
            suppress_tokens: suppress,
            suppress_blank: opts.suppress_blank,
            length_penalty: opts.length_penalty,
            repetition_penalty: opts.repetition_penalty,
            no_repeat_ngram_size: opts.no_repeat_ngram_size,
        };
        let result = dec.generate(enc, &req)?;
        let avg = avg_logprob(&result, opts.length_penalty);
        let ratio = compression_ratio(decode_text(tok, sp, &result.tokens)?.trim());
        let no_speech_prob = f64::from(result.no_speech_prob);

        let mut needs_fallback = false;
        let mut below_cr = false;
        if let Some(threshold) = opts.compression_ratio_threshold {
            if ratio > threshold {
                // Too repetitive.
                needs_fallback = true;
            } else {
                below_cr = true;
            }
        }
        let low_logprob = opts.log_prob_threshold.is_some_and(|thr| avg < thr);
        if low_logprob {
            needs_fallback = true;
        }
        if low_logprob
            && opts
                .no_speech_threshold
                .is_some_and(|thr| no_speech_prob > thr)
        {
            // Silence: the no-speech check after decoding handles it.
            needs_fallback = false;
        }

        let decoded = Decoded {
            result,
            avg_logprob: avg,
            temperature,
        };
        if !needs_fallback {
            return Ok(decoded);
        }
        all.push((decoded, below_cr));
    }

    // Every temperature failed. Python's `max` keeps the first of equal maxima; so does this.
    let pool: Vec<&Decoded> = if all.iter().any(|(_, below)| *below) {
        all.iter()
            .filter(|(_, below)| *below)
            .map(|(d, _)| d)
            .collect()
    } else {
        all.iter().map(|(d, _)| d).collect()
    };
    let mut best = pool
        .into_iter()
        .reduce(|best, d| {
            if d.avg_logprob > best.avg_logprob {
                d
            } else {
                best
            }
        })
        .cloned()
        .ok_or_else(|| EngineError::Decode("no decoding temperatures configured".to_owned()))?;
    best.temperature = last_temperature;
    Ok(best)
}

/// `round(max_initial_timestamp / time_precision)`, 50 for the default 1 s.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "clamped to a small non-negative value before the cast"
)]
fn max_initial_timestamp_index(seconds: f64) -> usize {
    // Python's `round` is half-to-even.
    (seconds / super::split::TIME_PRECISION)
        .round_ties_even()
        .clamp(0.0, 1500.0) as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn compression_ratio_matches_python() {
        let fixture: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/split_expected.json")).unwrap();
        for case in fixture["compression"].as_array().unwrap() {
            let text = case["text"].as_str().unwrap();
            let want = case["ratio"].as_f64().unwrap();
            let got = compression_ratio(text);
            assert!((got - want).abs() < 1e-12, "{text:?}: {got} != {want}");
        }
    }

    #[test]
    fn max_initial_timestamp_index_is_50() {
        assert_eq!(max_initial_timestamp_index(1.0), 50);
    }

    #[test]
    fn compression_ratio_thresholds() {
        assert!(compression_ratio(&"a".repeat(100)) > 2.4);
        assert!(compression_ratio("The quick brown fox jumps over the lazy dog.") < 2.4);
        // Exact Python values (`gen_split.py`).
        assert!((compression_ratio("hello hello hello hello") - 1.4375).abs() < 1e-12);
        assert!((compression_ratio(&"a".repeat(100)) - 100.0 / 12.0).abs() < 1e-12);
        assert!(compression_ratio("").abs() < 1e-12);
    }
}
