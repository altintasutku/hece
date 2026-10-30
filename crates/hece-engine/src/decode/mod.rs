//! Whisper segment decoding: the seek loop over 30 s windows, the previous-text prompt and
//! timestamp segmentation, written against the [`Decoder`] trait so it runs without a model.
//!
//! Ported from faster-whisper 1.2.1 (MIT), `transcribe.py` (`WhisperModel.generate_segments`,
//! `WhisperModel.get_prompt`).

mod fallback;
mod split;

pub use fallback::compression_ratio;
pub use split::{RawSegment, split_by_timestamps};

use hece_core::Segment;

use crate::error::EngineError;
use crate::mel::{Mel, N_FRAMES};
use crate::tokens::{SpecialTokens, TextTokenizer, sot_sequence};
use fallback::{Decoded, MAX_LENGTH, decode_text, generate_with_fallback};

/// Decoding options. `Default` is faster-whisper 1.2.1's `transcribe` defaults.
#[derive(Debug, Clone, PartialEq)]
pub struct DecodeOptions {
    /// Beam width at temperature 0.
    pub beam_size: usize,
    /// Sampled hypotheses per call at temperatures above 0.
    pub best_of: usize,
    /// Beam search patience.
    pub patience: f32,
    /// Exponent of the length normalisation of beam scores.
    pub length_penalty: f32,
    /// Penalty on repeated tokens (1 disables it).
    pub repetition_penalty: f32,
    /// Size of n-grams that may not repeat (0 disables it).
    pub no_repeat_ngram_size: usize,
    /// Temperatures tried in order until a result passes the thresholds.
    pub temperatures: Vec<f64>,
    /// A result whose text compresses better than this is treated as repetitive.
    pub compression_ratio_threshold: Option<f64>,
    /// A result whose average log probability is below this is treated as failed.
    pub log_prob_threshold: Option<f64>,
    /// A window with a no-speech probability above this (and a low log probability) is silent.
    pub no_speech_threshold: Option<f64>,
    /// Whether previous text is fed back as the prompt of the next window.
    pub condition_on_previous_text: bool,
    /// The previous-text prompt is dropped after a window decoded above this temperature.
    pub prompt_reset_on_temperature: f64,
    /// Whether blank outputs are suppressed at the first step.
    pub suppress_blank: bool,
    /// Configured suppress list (`-1` = the non-speech set). [`decode_segments`] takes the
    /// resolved list from [`crate::tokens::suppressed_tokens`].
    pub suppress_tokens: Vec<i32>,
    /// Latest allowed first timestamp, in seconds.
    pub max_initial_timestamp: f64,
}

impl Default for DecodeOptions {
    fn default() -> Self {
        Self {
            beam_size: 5,
            best_of: 5,
            patience: 1.0,
            length_penalty: 1.0,
            repetition_penalty: 1.0,
            no_repeat_ngram_size: 0,
            temperatures: vec![0.0, 0.2, 0.4, 0.6, 0.8, 1.0],
            compression_ratio_threshold: Some(2.4),
            log_prob_threshold: Some(-1.0),
            no_speech_threshold: Some(0.6),
            condition_on_previous_text: true,
            prompt_reset_on_temperature: 0.5,
            suppress_blank: true,
            suppress_tokens: vec![-1],
            max_initial_timestamp: 1.0,
        }
    }
}

/// One `generate` call, mirroring `CTranslate2`'s Whisper generation options.
#[derive(Debug, Clone, PartialEq)]
pub struct GenerateRequest<'a> {
    /// Decoder prompt (previous text, `sot_sequence`).
    pub prompt: &'a [u32],
    /// Beam width (1 = sampling or greedy).
    pub beam_size: usize,
    /// Beam search patience.
    pub patience: f32,
    /// Hypotheses to return; the first is used.
    pub num_hypotheses: usize,
    /// Sampling temperature.
    pub sampling_temperature: f32,
    /// Sample from the top k tokens (0 = full distribution, 1 = greedy).
    pub sampling_topk: usize,
    /// Maximum total length, prompt included.
    pub max_length: usize,
    /// Latest allowed first timestamp, as a timestamp index.
    pub max_initial_timestamp_index: usize,
    /// Token ids never generated.
    pub suppress_tokens: &'a [i32],
    /// Whether blank outputs are suppressed at the first step.
    pub suppress_blank: bool,
    /// Exponent of the length normalisation of beam scores.
    pub length_penalty: f32,
    /// Penalty on repeated tokens.
    pub repetition_penalty: f32,
    /// Size of n-grams that may not repeat.
    pub no_repeat_ngram_size: usize,
}

/// The first hypothesis of a `generate` call.
#[derive(Debug, Clone, PartialEq)]
pub struct GenerateResult {
    /// Generated ids, without the prompt; may contain timestamp tokens.
    pub tokens: Vec<u32>,
    /// Length-normalised cumulative log probability.
    pub score: f32,
    /// Probability of `<|nospeech|>` at the first step.
    pub no_speech_prob: f32,
}

/// The model calls the decode loop needs.
pub trait Decoder {
    /// Encoder output for one window.
    type Encoded;
    /// Encodes one `[n_mels][3000]` mel window.
    fn encode(&mut self, window: &[f32]) -> Result<Self::Encoded, EngineError>;
    /// Generates the first hypothesis for `req`.
    fn generate(
        &mut self,
        enc: &Self::Encoded,
        req: &GenerateRequest<'_>,
    ) -> Result<GenerateResult, EngineError>;
}

/// Decodes `mel` into segments with fixed `language`. `initial_prompt` are prompt tokens (see
/// [`crate::tokens::initial_prompt_tokens`]); `suppress` is the resolved suppress list.
#[expect(
    clippy::too_many_arguments,
    reason = "mirrors the reference inputs; bundling them adds a type with no other use"
)]
pub fn decode_segments<D: Decoder>(
    dec: &mut D,
    tok: &dyn TextTokenizer,
    sp: &SpecialTokens,
    language: u32,
    mel: &Mel,
    initial_prompt: &[u32],
    suppress: &[i32],
    opts: &DecodeOptions,
) -> Result<Vec<Segment>, EngineError> {
    let content_frames = mel.frames.saturating_sub(1);
    let sot_seq = sot_sequence(sp, language);
    let mut seek = 0;
    let mut all_tokens: Vec<u32> = initial_prompt.to_vec();
    let mut prompt_reset_since = 0;
    let mut segments = Vec::new();

    while seek < content_frames {
        let time_offset = frames_to_secs(seek);
        let segment_size = N_FRAMES.min(content_frames - seek);
        let segment_duration = frames_to_secs(segment_size);

        let enc = dec.encode(&mel.window(seek, segment_size))?;
        let prompt = get_prompt(sp.sot_prev, &all_tokens[prompt_reset_since..], &sot_seq);
        let decoded = generate_with_fallback(dec, &enc, &prompt, tok, sp, suppress, opts)?;

        if let Some(threshold) = opts.no_speech_threshold {
            let silent = f64::from(decoded.result.no_speech_prob) > threshold
                && !opts
                    .log_prob_threshold
                    .is_some_and(|thr| decoded.avg_logprob > thr);
            if silent {
                seek += segment_size;
                continue;
            }
        }

        let previous_seek = seek;
        let (current, next_seek, _single_ending) = split_by_timestamps(
            sp.timestamp_begin,
            &decoded.result.tokens,
            time_offset,
            segment_size,
            segment_duration,
            seek,
        );
        // The reference loops forever when a window ends on a consecutive timestamp pair at
        // position 0 (seek += 0, same input, same output). Move to the next window instead.
        seek = if next_seek > previous_seek {
            next_seek
        } else {
            previous_seek + segment_size
        };

        for raw in current {
            let text = decode_text(tok, sp, &raw.tokens)?;
            #[expect(
                clippy::float_cmp,
                reason = "faster-whisper drops segments whose times are exactly equal"
            )]
            let empty_span = raw.start == raw.end;
            if empty_span || text.trim().is_empty() {
                continue;
            }
            all_tokens.extend_from_slice(&raw.tokens);
            segments.push(to_segment(text, &raw, &decoded));
        }

        if !opts.condition_on_previous_text
            || decoded.temperature > opts.prompt_reset_on_temperature
        {
            prompt_reset_since = all_tokens.len();
        }
    }
    Ok(segments)
}

/// `[sot_prev, last 223 previous tokens]` (when there are any) followed by `sot_sequence`.
/// Port of `get_prompt` without hotwords, prefix or `without_timestamps`.
fn get_prompt(sot_prev: u32, previous: &[u32], sot_seq: &[u32; 3]) -> Vec<u32> {
    let mut prompt = Vec::with_capacity(1 + MAX_PREVIOUS + sot_seq.len());
    if !previous.is_empty() {
        prompt.push(sot_prev);
        prompt.extend_from_slice(&previous[previous.len().saturating_sub(MAX_PREVIOUS)..]);
    }
    prompt.extend_from_slice(sot_seq);
    prompt
}

/// Previous-text tokens kept in the prompt (`max_length // 2 - 1`).
const MAX_PREVIOUS: usize = MAX_LENGTH / 2 - 1;

/// Seconds per mel frame (`hop_length / sampling_rate`).
const TIME_PER_FRAME: f64 = 160.0 / 16_000.0;

#[expect(
    clippy::cast_precision_loss,
    reason = "frame counts of any realistic recording are far below 2^52"
)]
fn frames_to_secs(frames: usize) -> f64 {
    frames as f64 * TIME_PER_FRAME
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "hece_core::Segment stores f32; the reference computes in f64 and so does this"
)]
fn to_segment(text: String, raw: &RawSegment, decoded: &Decoded) -> Segment {
    Segment {
        text,
        start_s: raw.start as f32,
        end_s: raw.end as f32,
        no_speech_prob: decoded.result.no_speech_prob,
        avg_logprob: decoded.avg_logprob as f32,
    }
}

#[cfg(test)]
#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    reason = "test data uses small token counts and frame indices"
)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    const EOT: u32 = 50_257;
    const SOT: u32 = 50_258;
    const LANG: u32 = 50_268;
    const TRANSCRIBE: u32 = 50_360;
    const SOT_PREV: u32 = 50_362;
    const TB: u32 = 50_365;

    fn sp() -> SpecialTokens {
        SpecialTokens {
            sot: SOT,
            sot_prev: SOT_PREV,
            sot_lm: 50_361,
            eot: EOT,
            transcribe: TRANSCRIBE,
            translate: 50_359,
            no_timestamps: 50_364,
            no_speech: 50_363,
            timestamp_begin: TB,
        }
    }

    fn t(n: u32) -> u32 {
        TB + n
    }

    const SOT_SEQ: [u32; 3] = [SOT, LANG, TRANSCRIBE];

    /// Renders text ids as ` w<id>` and anything at or above `eot` as `<|id|>`, so leaked
    /// timestamp ids are visible in segment text.
    struct FakeTok;

    impl TextTokenizer for FakeTok {
        fn encode(&self, _text: &str) -> Result<Vec<u32>, EngineError> {
            Ok(Vec::new())
        }
        fn decode(&self, ids: &[u32]) -> Result<String, EngineError> {
            Ok(ids
                .iter()
                .map(|&i| {
                    if i >= EOT {
                        format!("<|{i}|>")
                    } else {
                        format!(" w{i}")
                    }
                })
                .collect())
        }
        fn token_id(&self, _token: &str) -> Option<u32> {
            None
        }
    }

    #[derive(Debug, Clone)]
    struct Call {
        prompt: Vec<u32>,
        window_start: f32,
        beam_size: usize,
        patience: f32,
        num_hypotheses: usize,
        sampling_temperature: f32,
        sampling_topk: usize,
        max_length: usize,
        max_initial_timestamp_index: usize,
        suppress: Vec<i32>,
        suppress_blank: bool,
    }

    /// Scripted decoder: pops one queued result per `generate` and records every request.
    /// `encode` returns the first value of the window, which the test mel sets to the column
    /// index, so it reveals the seek.
    struct FakeDecoder {
        results: VecDeque<Result<GenerateResult, EngineError>>,
        calls: Vec<Call>,
        encodes: usize,
    }

    impl FakeDecoder {
        fn new(results: Vec<GenerateResult>) -> Self {
            Self {
                results: results.into_iter().map(Ok).collect(),
                calls: Vec::new(),
                encodes: 0,
            }
        }
    }

    impl Decoder for FakeDecoder {
        type Encoded = f32;
        fn encode(&mut self, window: &[f32]) -> Result<f32, EngineError> {
            assert_eq!(window.len(), 2 * 3000);
            self.encodes += 1;
            Ok(window[0])
        }
        fn generate(
            &mut self,
            enc: &f32,
            req: &GenerateRequest<'_>,
        ) -> Result<GenerateResult, EngineError> {
            self.calls.push(Call {
                prompt: req.prompt.to_vec(),
                window_start: *enc,
                beam_size: req.beam_size,
                patience: req.patience,
                num_hypotheses: req.num_hypotheses,
                sampling_temperature: req.sampling_temperature,
                sampling_topk: req.sampling_topk,
                max_length: req.max_length,
                max_initial_timestamp_index: req.max_initial_timestamp_index,
                suppress: req.suppress_tokens.to_vec(),
                suppress_blank: req.suppress_blank,
            });
            self.results
                .pop_front()
                .unwrap_or_else(|| Err(EngineError::Decode("script exhausted".to_owned())))
        }
    }

    /// A result whose recovered `avg_logprob` is `avg` (`length_penalty` 1).
    fn res(tokens: Vec<u32>, avg: f64, no_speech_prob: f32) -> GenerateResult {
        let len = tokens.len() as f64;
        let score = (avg * (len + 1.0) / len) as f32;
        GenerateResult {
            tokens,
            score,
            no_speech_prob,
        }
    }

    /// Text that compresses far beyond 2.4 when rendered by `FakeTok`.
    fn repetitive() -> Vec<u32> {
        let mut v = vec![t(0)];
        v.extend(std::iter::repeat_n(100, 60));
        v.push(t(500));
        v
    }

    /// `seconds` of mel (`frames = seconds * 100 + 1`); every value is its column index.
    fn mel(seconds: usize) -> Mel {
        let frames = seconds * 100 + 1;
        let data = (0..2).flat_map(|_| (0..frames).map(|f| f as f32)).collect();
        Mel {
            n_mels: 2,
            frames,
            data,
        }
    }

    fn run(dec: &mut FakeDecoder, mel: &Mel, initial: &[u32]) -> Vec<Segment> {
        decode_segments(
            dec,
            &FakeTok,
            &sp(),
            LANG,
            mel,
            initial,
            &[1, 2, 3],
            &DecodeOptions::default(),
        )
        .unwrap()
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn defaults_match_faster_whisper() {
        let o = DecodeOptions::default();
        assert_eq!((o.beam_size, o.best_of, o.no_repeat_ngram_size), (5, 5, 0));
        assert_eq!(o.temperatures, [0.0, 0.2, 0.4, 0.6, 0.8, 1.0]);
        assert_eq!(o.compression_ratio_threshold, Some(2.4));
        assert_eq!(o.log_prob_threshold, Some(-1.0));
        assert_eq!(o.no_speech_threshold, Some(0.6));
        assert!(o.condition_on_previous_text && o.suppress_blank);
        assert_eq!(o.suppress_tokens, [-1]);
    }

    #[test]
    fn one_window_segments_from_timestamps() {
        let mut dec = FakeDecoder::new(vec![res(
            vec![t(0), 100, 101, t(250), t(250), 102, t(400), 103, t(500)],
            -0.2,
            0.01,
        )]);
        let segs = run(&mut dec, &mel(15), &[]);
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].text, " w100 w101");
        assert!(close(segs[0].start_s, 0.0) && close(segs[0].end_s, 5.0));
        // A timestamp inside the segment must not leak into the text.
        assert_eq!(segs[1].text, " w102 w103");
        assert!(close(segs[1].start_s, 5.0) && close(segs[1].end_s, 10.0));
        assert!(segs.iter().all(|s| !s.text.contains("<|")));
        assert!(close(segs[0].avg_logprob, -0.2) && close(segs[0].no_speech_prob, 0.01));

        assert_eq!(dec.calls.len(), 1);
        let c = &dec.calls[0];
        assert_eq!(c.prompt, SOT_SEQ);
        assert_eq!((c.beam_size, c.num_hypotheses, c.sampling_topk), (5, 1, 1));
        assert!(close(c.patience, 1.0));
        assert_eq!((c.max_length, c.max_initial_timestamp_index), (448, 50));
        assert_eq!(c.suppress, [1, 2, 3]);
        assert!(c.suppress_blank);
    }

    #[test]
    fn repetitive_result_falls_back_to_sampling() {
        let mut dec = FakeDecoder::new(vec![
            res(repetitive(), -0.1, 0.01),
            res(vec![t(0), 104, t(500)], -0.3, 0.01),
        ]);
        let segs = run(&mut dec, &mel(15), &[]);
        assert_eq!(dec.calls.len(), 2);
        let c = &dec.calls[1];
        assert_eq!((c.beam_size, c.num_hypotheses, c.sampling_topk), (1, 5, 0));
        assert!(close(c.sampling_temperature, 0.2));
        assert_eq!(c.prompt, SOT_SEQ);
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].text, " w104");
        assert!(close(segs[0].avg_logprob, -0.3));
    }

    #[test]
    fn all_temperatures_fail_picks_best_below_threshold_and_resets_prompt() {
        let first = vec![t(0), 100, t(400), t(400), 101, t(1000)];
        let mut dec = FakeDecoder::new(vec![
            res(repetitive(), -0.2, 0.1),
            res(vec![t(0), 110, t(1000)], -1.2, 0.1),
            res(first, -1.1, 0.1),
            res(vec![t(0), 130, t(1000)], -1.3, 0.1),
            res(vec![t(0), 140, t(1000)], -1.4, 0.1),
            res(vec![t(0), 150, t(1000)], -1.5, 0.1),
            res(vec![t(0), 160, t(500)], -0.2, 0.1),
        ]);
        let segs = run(&mut dec, &mel(45), &[]);
        assert_eq!(dec.calls.len(), 7);
        let temps: Vec<f32> = dec.calls[1..6]
            .iter()
            .map(|c| c.sampling_temperature)
            .collect();
        for (got, want) in temps.iter().zip([0.2, 0.4, 0.6, 0.8, 1.0]) {
            assert!(close(*got, want));
        }
        // The -1.1 result (temperature 0.4) wins; the repetitive -0.2 one is excluded.
        assert_eq!(segs[0].text, " w100");
        assert_eq!(segs[1].text, " w101");
        assert!(close(segs[0].avg_logprob, -1.1));
        // The reported temperature is 1.0 > 0.5, so the next window gets no previous text.
        assert_eq!(dec.calls[6].prompt, SOT_SEQ);
        assert!(close(dec.calls[6].window_start, 3000.0));
        assert_eq!(segs.len(), 3);
        assert!(close(segs[2].start_s, 30.0) && close(segs[2].end_s, 40.0));
    }

    #[test]
    fn all_fail_above_compression_threshold_picks_best_overall() {
        let mut worse = repetitive();
        worse[1] = 101;
        let mut dec = FakeDecoder::new(vec![
            res(worse, -0.5, 0.1),
            res(repetitive(), -0.2, 0.1),
            res(repetitive(), -0.9, 0.1),
            res(repetitive(), -0.9, 0.1),
            res(repetitive(), -0.9, 0.1),
            res(repetitive(), -0.9, 0.1),
        ]);
        let segs = run(&mut dec, &mel(15), &[]);
        assert_eq!(dec.calls.len(), 6);
        assert_eq!(segs.len(), 1);
        assert!(segs[0].text.starts_with(" w100 w100"));
        assert!(close(segs[0].avg_logprob, -0.2));
    }

    #[test]
    fn silent_window_is_skipped() {
        let mut dec = FakeDecoder::new(vec![res(vec![t(0), 100, t(500)], -1.5, 0.9)]);
        let segs = run(&mut dec, &mel(15), &[]);
        assert_eq!(dec.calls.len(), 1);
        assert!(segs.is_empty());
    }

    #[test]
    fn long_audio_seeks_to_last_timestamp_and_carries_prompt() {
        let first = vec![t(0), 100, 101, t(600), t(600), 102, t(1200), t(1200), 103];
        let mut dec = FakeDecoder::new(vec![
            res(first, -0.2, 0.01),
            res(vec![t(0), 104, t(500)], -0.2, 0.01),
        ]);
        let segs = run(&mut dec, &mel(45), &[]);
        assert_eq!(dec.calls.len(), 2);
        assert!(close(dec.calls[0].window_start, 0.0));
        // seek = 1200 * input_stride 2 = 2400 frames.
        assert!(close(dec.calls[1].window_start, 2400.0));
        let mut want = vec![SOT_PREV, t(0), 100, 101, t(600), t(600), 102, t(1200)];
        want.extend(SOT_SEQ);
        assert_eq!(dec.calls[1].prompt, want);
        assert_eq!(segs.len(), 3);
        assert!(close(segs[1].start_s, 12.0) && close(segs[1].end_s, 24.0));
        assert_eq!(segs[2].text, " w104");
        assert!(close(segs[2].start_s, 24.0) && close(segs[2].end_s, 34.0));
    }

    #[test]
    fn initial_prompt_follows_sot_prev() {
        let mut dec = FakeDecoder::new(vec![res(vec![t(0), 100, t(500)], -0.2, 0.01)]);
        run(&mut dec, &mel(15), &[7, 8, 9]);
        assert_eq!(
            dec.calls[0].prompt,
            [SOT_PREV, 7, 8, 9, SOT, LANG, TRANSCRIBE]
        );
    }

    #[test]
    fn previous_text_is_capped_at_223_tokens() {
        let initial: Vec<u32> = (0..300).collect();
        let mut dec = FakeDecoder::new(vec![res(vec![t(0), 100, t(500)], -0.2, 0.01)]);
        run(&mut dec, &mel(15), &initial);
        let prompt = &dec.calls[0].prompt;
        assert_eq!(prompt.len(), 1 + 223 + 3);
        assert_eq!(prompt[1], 77);
        assert_eq!(prompt[223], 299);
    }

    #[test]
    fn empty_and_zero_length_segments_are_dropped() {
        // [t0, t0] has start == end; [t0, t(300)] alone has no text.
        let mut dec = FakeDecoder::new(vec![res(
            vec![t(0), t(0), t(0), t(300), t(300), 100, t(500)],
            -0.2,
            0.01,
        )]);
        let segs = run(&mut dec, &mel(15), &[]);
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].text, " w100");
    }

    #[test]
    fn window_without_progress_still_terminates() {
        // Ends on a consecutive pair at position 0: the reference would seek by 0 forever.
        let mut dec = FakeDecoder::new(vec![res(vec![t(0), 100, t(0), t(0)], -0.2, 0.01)]);
        let segs = run(&mut dec, &mel(15), &[]);
        assert_eq!(dec.calls.len(), 1);
        assert!(segs.is_empty());
    }

    #[test]
    fn decoder_errors_propagate() {
        let mut dec = FakeDecoder::new(Vec::new());
        let err = decode_segments(
            &mut dec,
            &FakeTok,
            &sp(),
            LANG,
            &mel(15),
            &[],
            &[],
            &DecodeOptions::default(),
        )
        .unwrap_err();
        assert_eq!(err.code(), "decode_failed");
    }

    #[test]
    fn empty_mel_decodes_nothing() {
        let mut dec = FakeDecoder::new(Vec::new());
        let empty = Mel {
            n_mels: 2,
            frames: 0,
            data: Vec::new(),
        };
        assert!(run(&mut dec, &empty, &[]).is_empty());
        assert_eq!(dec.encodes, 0);
    }
}
