//! The transcription pipeline: VAD, chunk collection, log-mel, segment decoding and timestamp
//! restoration, written against [`Decoder`] so it runs without a model.
//!
//! Ported from faster-whisper 1.2.1 (MIT), `transcribe.py` (`WhisperModel.transcribe`,
//! `get_suppressed_tokens`, `restore_speech_timestamps`), without word timestamps, clip
//! timestamps, hotwords or batching.

use std::collections::BTreeSet;

use hece_core::Segment;

use crate::audio::to_f32;
use crate::decode::{DecodeOptions, Decoder, decode_segments_restored};
use crate::error::EngineError;
use crate::mel::{Mel, MelExtractor};
use crate::tokens::{SpecialTokens, TextTokenizer, initial_prompt_tokens, suppressed_tokens};
#[cfg(test)]
use crate::vad::SpeechSpan;
use crate::vad::{SileroVad, SpeechTimestampsMap, VadOptions, collect_chunks};

/// Options of one transcription. `Default` is faster-whisper 1.2.1's `transcribe` defaults with
/// `vad_filter=True`.
#[derive(Debug, Clone, PartialEq)]
pub struct TranscribeOptions {
    /// Language code such as `tr`; `None` detects it from the first 30 s window.
    pub language: Option<String>,
    /// Text fed to the decoder as previous context (e.g. a vocabulary prompt).
    pub initial_prompt: Option<String>,
    /// Whether non-speech audio is removed with Silero VAD before decoding.
    pub vad: bool,
    /// VAD options, used when `vad` is set.
    pub vad_options: VadOptions,
    /// Decoding options.
    pub decode: DecodeOptions,
}

impl Default for TranscribeOptions {
    fn default() -> Self {
        Self {
            language: None,
            initial_prompt: None,
            vad: true,
            vad_options: VadOptions::default(),
            decode: DecodeOptions::default(),
        }
    }
}

/// Weight and activation precision of the `CTranslate2` model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ComputeKind {
    /// 8-bit weights, 16-bit float activations.
    #[default]
    Int8Float16,
    /// 16-bit float.
    Float16,
    /// 8-bit weights and activations.
    Int8,
    /// 32-bit float.
    Float32,
}

/// Audio ready for decoding: its log-mel and, when VAD ran, the map back to original times.
#[derive(Debug)]
pub(crate) struct Prepared {
    pub(crate) mel: Mel,
    pub(crate) map: Option<SpeechTimestampsMap>,
}

/// Converts, VAD-filters and featurises `audio`. `None` when VAD finds no speech: the reference
/// then decodes an empty spectrogram, which yields no segments, so the decoder is skipped.
pub(crate) fn prepare(
    vad: &mut SileroVad,
    mel: &MelExtractor,
    audio: &[i16],
    opts: &TranscribeOptions,
) -> Result<Option<Prepared>, EngineError> {
    let samples = to_f32(audio);
    if !opts.vad {
        return Ok(Some(Prepared {
            mel: mel.log_mel(&samples),
            map: None,
        }));
    }
    let spans = vad.speech_spans(&samples, &opts.vad_options)?;
    if spans.is_empty() {
        return Ok(None);
    }
    let speech = collect_chunks(&samples, &spans);
    Ok(Some(Prepared {
        mel: mel.log_mel(&speech),
        map: Some(SpeechTimestampsMap::new(&spans)),
    }))
}

/// Decodes prepared audio with fixed `language` and restores the original segment times.
pub(crate) fn decode_prepared<D: Decoder>(
    dec: &mut D,
    tok: &dyn TextTokenizer,
    sp: &SpecialTokens,
    language: u32,
    prepared: &Prepared,
    opts: &TranscribeOptions,
) -> Result<Vec<Segment>, EngineError> {
    let initial = match &opts.initial_prompt {
        Some(prompt) => initial_prompt_tokens(tok, prompt)?,
        None => Vec::new(),
    };
    let suppress = resolve_suppress(tok, sp, &opts.decode.suppress_tokens)?;
    let map = prepared.map.as_ref();
    decode_segments_restored(
        dec,
        tok,
        sp,
        language,
        &prepared.mel,
        &initial,
        &suppress,
        &opts.decode,
        &|t, is_end| restore(map, t, is_end),
    )
}

/// Transcribes 16 kHz mono `audio` into segments with fixed `language` (a language token id).
/// With `opts.vad`, audio without speech returns no segments without calling the decoder.
#[expect(
    clippy::too_many_arguments,
    reason = "the pipeline's parts are separate so tests can swap the decoder and tokenizer"
)]
pub fn transcribe_with<D: Decoder>(
    dec: &mut D,
    vad: &mut SileroVad,
    mel: &MelExtractor,
    tok: &dyn TextTokenizer,
    sp: &SpecialTokens,
    language: u32,
    audio: &[i16],
    opts: &TranscribeOptions,
) -> Result<Vec<Segment>, EngineError> {
    match prepare(vad, mel, audio, opts)? {
        Some(prepared) => decode_prepared(dec, tok, sp, language, &prepared, opts),
        None => Ok(Vec::new()),
    }
}

/// Port of `get_suppressed_tokens`: `-1` expands to the non-speech set, any non-empty list gets
/// the control tokens, and the result is sorted and deduplicated. An empty list stays empty,
/// because `transcribe` only calls `get_suppressed_tokens` for a non-empty list.
fn resolve_suppress(
    tok: &dyn TextTokenizer,
    sp: &SpecialTokens,
    configured: &[i32],
) -> Result<Vec<i32>, EngineError> {
    if configured.is_empty() {
        return Ok(Vec::new());
    }
    let explicit = configured.iter().copied().filter(|&t| t >= 0);
    if configured.contains(&-1) {
        // `suppressed_tokens` already adds the control tokens.
        let mut set: BTreeSet<i32> = suppressed_tokens(tok, sp)?.into_iter().collect();
        set.extend(explicit);
        return Ok(set.into_iter().collect());
    }
    let mut set: BTreeSet<i32> = explicit.collect();
    for id in [
        sp.transcribe,
        sp.translate,
        sp.sot,
        sp.sot_prev,
        sp.sot_lm,
        sp.no_speech,
    ] {
        set.insert(
            i32::try_from(id)
                .map_err(|_| EngineError::Tokenizer("token id exceeds i32".to_owned()))?,
        );
    }
    Ok(set.into_iter().collect())
}

/// Original-audio time of `t`; identity without VAD.
fn restore(map: Option<&SpeechTimestampsMap>, t: f64, is_end: bool) -> f64 {
    map.map_or(t, |m| m.original_time(t, is_end))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode::{GenerateRequest, GenerateResult};

    const EOT: u32 = 50_257;
    const TB: u32 = 50_365;

    fn sp() -> SpecialTokens {
        SpecialTokens {
            sot: 50_258,
            sot_prev: 50_362,
            sot_lm: 50_361,
            eot: EOT,
            transcribe: 50_360,
            translate: 50_359,
            no_timestamps: 50_364,
            no_speech: 50_363,
            timestamp_begin: TB,
        }
    }

    /// One token per character (its code point modulo 1000); decodes ids as ` w<id>`.
    struct FakeTok;

    impl TextTokenizer for FakeTok {
        fn encode(&self, text: &str) -> Result<Vec<u32>, EngineError> {
            Ok(text.chars().map(|c| u32::from(c) % 1000).collect())
        }
        fn decode(&self, ids: &[u32]) -> Result<String, EngineError> {
            Ok(ids.iter().fold(String::new(), |mut out, i| {
                out.push_str(" w");
                out.push_str(&i.to_string());
                out
            }))
        }
        fn token_id(&self, _token: &str) -> Option<u32> {
            None
        }
    }

    /// Counts calls; every `generate` returns one 0-1 s segment with text `w100`.
    #[derive(Default)]
    struct CountingDecoder {
        encodes: usize,
        prompts: Vec<Vec<u32>>,
        suppress: Vec<Vec<i32>>,
    }

    impl Decoder for CountingDecoder {
        type Encoded = ();
        fn encode(&mut self, window: &[f32]) -> Result<(), EngineError> {
            assert_eq!(window.len(), 128 * 3000);
            self.encodes += 1;
            Ok(())
        }
        fn generate(
            &mut self,
            _enc: &(),
            req: &GenerateRequest<'_>,
        ) -> Result<GenerateResult, EngineError> {
            self.prompts.push(req.prompt.to_vec());
            self.suppress.push(req.suppress_tokens.to_vec());
            Ok(GenerateResult {
                tokens: vec![TB, 100, TB + 50],
                score: -0.1,
                no_speech_prob: 0.01,
            })
        }
    }

    fn run(dec: &mut CountingDecoder, audio: &[i16], opts: &TranscribeOptions) -> Vec<Segment> {
        let mut vad = SileroVad::new().unwrap();
        let mel = MelExtractor::new(128);
        transcribe_with(dec, &mut vad, &mel, &FakeTok, &sp(), 50_268, audio, opts).unwrap()
    }

    #[test]
    fn defaults() {
        let o = TranscribeOptions::default();
        assert_eq!(o.language, None);
        assert_eq!(o.initial_prompt, None);
        assert!(o.vad);
        assert_eq!(o.vad_options, VadOptions::default());
        assert_eq!(o.decode, DecodeOptions::default());
        assert_eq!(ComputeKind::default(), ComputeKind::Int8Float16);
    }

    #[test]
    fn silent_audio_with_vad_never_reaches_the_decoder() {
        let mut dec = CountingDecoder::default();
        let segs = run(&mut dec, &vec![0; 32_000], &TranscribeOptions::default());
        assert!(segs.is_empty());
        assert_eq!(dec.encodes, 0);
        assert!(dec.prompts.is_empty());
    }

    #[test]
    fn empty_audio_with_vad_never_reaches_the_decoder() {
        let mut dec = CountingDecoder::default();
        assert!(run(&mut dec, &[], &TranscribeOptions::default()).is_empty());
        assert_eq!(dec.encodes, 0);
    }

    #[test]
    fn without_vad_the_decoder_runs() {
        let mut dec = CountingDecoder::default();
        let opts = TranscribeOptions {
            vad: false,
            initial_prompt: Some("  ab ".to_owned()),
            ..TranscribeOptions::default()
        };
        let segs = run(&mut dec, &vec![0; 32_000], &opts);
        assert_eq!(dec.encodes, 1);
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].text, " w100");
        assert!((segs[0].end_s - 1.0).abs() < 1e-6);
        // `" ab"` follows `sot_prev`, then `sot, language, transcribe`.
        assert_eq!(dec.prompts[0], [50_362, 32, 97, 98, 50_258, 50_268, 50_360]);
        // The default `[-1]` resolves to the non-speech set plus the control tokens.
        let suppress = &dec.suppress[0];
        assert!(suppress.windows(2).all(|w| w[0] < w[1]));
        assert!(suppress.contains(&50_360) && suppress.contains(&50_363));
        assert!(!suppress.contains(&-1));
    }

    #[test]
    fn explicit_and_empty_suppress_lists() {
        let tok = FakeTok;
        let explicit = resolve_suppress(&tok, &sp(), &[7, 3, 7]).unwrap();
        assert_eq!(
            explicit,
            [3, 7, 50_258, 50_359, 50_360, 50_361, 50_362, 50_363]
        );
        // faster-whisper passes an empty list through untouched.
        assert!(resolve_suppress(&tok, &sp(), &[]).unwrap().is_empty());
        let mixed = resolve_suppress(&tok, &sp(), &[-1, 5]).unwrap();
        assert!(mixed.contains(&5) && !mixed.contains(&-1));
    }

    #[test]
    fn restored_times_skip_removed_silence() {
        // Two spans: [1 s, 2 s) and [5 s, 6 s); 3 s of silence sit before the second one.
        let spans = [
            SpeechSpan {
                start: 16_000,
                end: 32_000,
            },
            SpeechSpan {
                start: 80_000,
                end: 96_000,
            },
        ];
        let map = SpeechTimestampsMap::new(&spans);
        assert!((restore(Some(&map), 0.5, false) - 1.5).abs() < 1e-9);
        // Exactly on the first chunk's end: an end time stays in the first chunk.
        assert!((restore(Some(&map), 1.0, true) - 2.0).abs() < 1e-9);
        assert!((restore(Some(&map), 1.0, false) - 5.0).abs() < 1e-9);
        assert!((restore(None, 1.234, true) - 1.234).abs() < 1e-12);
    }
}
