//! `CTranslate2` backend: [`Decoder`] over `ct2rs::sys::Whisper` and the [`LocalEngine`] that
//! wires it into the pipeline.
//!
//! Ported from faster-whisper 1.2.1 (MIT), `transcribe.py` (`WhisperModel.__init__`,
//! `WhisperModel.encode`, `WhisperModel.detect_language`, the `generate` call of
//! `generate_with_fallback`).

use std::path::Path;

use ct2rs::sys::{StorageView, Whisper, WhisperOptions};
use ct2rs::{ComputeType, Config, Device};
use hece_core::{Transcript, Vocabulary};
use tokenizers::Tokenizer;

use crate::decode::{Decoder, GenerateRequest, GenerateResult};
use crate::engine::{ComputeKind, TranscribeOptions, decode_prepared, prepare};
use crate::error::EngineError;
use crate::mel::{MelExtractor, N_FRAMES};
use crate::tokens::{SpecialTokens, language_token, vocabulary_prompt};
use crate::vad::SileroVad;

/// Files a converted Whisper model directory must contain.
const REQUIRED_FILES: [&str; 3] = ["model.bin", "tokenizer.json", "config.json"];

/// Provider name reported in transcripts.
const PROVIDER: &str = "local-ct2";

fn decode_err(e: impl std::fmt::Display) -> EngineError {
    EngineError::Decode(e.to_string())
}

impl From<ComputeKind> for ComputeType {
    fn from(kind: ComputeKind) -> Self {
        match kind {
            ComputeKind::Int8Float16 => Self::INT8_FLOAT16,
            ComputeKind::Float16 => Self::FLOAT16,
            ComputeKind::Int8 => Self::INT8,
            ComputeKind::Float32 => Self::FLOAT32,
        }
    }
}

/// [`Decoder`] borrowing the model and tokenizer of a [`LocalEngine`] for one transcription.
struct Ct2Decoder<'a> {
    whisper: &'a Whisper,
    tokenizer: &'a Tokenizer,
    n_mels: usize,
}

/// A `[1, n_mels, 3000]` CPU view over `window`.
fn features(n_mels: usize, window: &mut [f32]) -> Result<StorageView<'_>, EngineError> {
    StorageView::new(&[1, n_mels, N_FRAMES], window, Device::CPU).map_err(decode_err)
}

impl Decoder for Ct2Decoder<'_> {
    /// Encoder output kept on the model's device; `generate` accepts it in place of features.
    type Encoded = StorageView<'static>;

    fn encode(&mut self, window: &[f32]) -> Result<Self::Encoded, EngineError> {
        let mut buf = window.to_vec();
        let features = features(self.n_mels, &mut buf)?;
        self.whisper.encode(&features, false).map_err(decode_err)
    }

    fn generate(
        &mut self,
        enc: &Self::Encoded,
        req: &GenerateRequest<'_>,
    ) -> Result<GenerateResult, EngineError> {
        // ct2rs takes the prompt as token strings; CTranslate2 maps them back to the same ids.
        let prompt = req
            .prompt
            .iter()
            .map(|&id| {
                self.tokenizer
                    .id_to_token(id)
                    .ok_or_else(|| EngineError::Tokenizer(format!("unknown token id {id}")))
            })
            .collect::<Result<Vec<String>, EngineError>>()?;
        let options = WhisperOptions {
            beam_size: req.beam_size,
            patience: req.patience,
            length_penalty: req.length_penalty,
            repetition_penalty: req.repetition_penalty,
            no_repeat_ngram_size: req.no_repeat_ngram_size,
            max_length: req.max_length,
            sampling_topk: req.sampling_topk,
            sampling_temperature: req.sampling_temperature,
            num_hypotheses: req.num_hypotheses,
            return_scores: true,
            return_logits_vocab: false,
            return_no_speech_prob: true,
            max_initial_timestamp_index: req.max_initial_timestamp_index,
            suppress_blank: req.suppress_blank,
            suppress_tokens: req.suppress_tokens.to_vec(),
        };
        let result = self
            .whisper
            .generate(enc, &[prompt], &options)
            .map_err(decode_err)?
            .into_iter()
            .next()
            .ok_or_else(|| EngineError::Decode("generate returned no result".to_owned()))?;
        let tokens = result
            .sequences_ids
            .first()
            .ok_or_else(|| EngineError::Decode("generate returned no hypothesis".to_owned()))?
            .iter()
            .map(|&id| {
                u32::try_from(id)
                    .map_err(|_| EngineError::Decode("token id exceeds u32".to_owned()))
            })
            .collect::<Result<Vec<u32>, EngineError>>()?;
        let score = result
            .scores
            .first()
            .copied()
            .ok_or_else(|| EngineError::Decode("generate returned no score".to_owned()))?;
        Ok(GenerateResult {
            tokens,
            score,
            no_speech_prob: result.no_speech_prob,
        })
    }
}

/// Local Whisper on `CTranslate2` with CUDA: model, tokenizer, VAD and mel extractor.
pub struct LocalEngine {
    whisper: Whisper,
    tokenizer: Tokenizer,
    special: SpecialTokens,
    mel: MelExtractor,
    vad: SileroVad,
}

impl std::fmt::Debug for LocalEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalEngine")
            .field("whisper", &self.whisper)
            .finish_non_exhaustive()
    }
}

impl LocalEngine {
    /// Loads a converted Whisper model from `model_dir` on CUDA device `device_index`.
    ///
    /// Missing files or an unloadable model give `ModelDirInvalid`; a failing CUDA runtime or
    /// device gives `CudaUnavailable`.
    pub fn load(
        model_dir: &Path,
        compute: ComputeKind,
        device_index: i32,
    ) -> Result<Self, EngineError> {
        for file in REQUIRED_FILES {
            if !model_dir.join(file).is_file() {
                return Err(EngineError::ModelDirInvalid {
                    reason: format!("missing {file}"),
                });
            }
        }
        let tokenizer = Tokenizer::from_file(model_dir.join("tokenizer.json")).map_err(|e| {
            EngineError::ModelDirInvalid {
                reason: format!("unreadable tokenizer.json: {e}"),
            }
        })?;
        let special = SpecialTokens::resolve(&tokenizer)?;
        let config = Config {
            device: Device::CUDA,
            compute_type: compute.into(),
            device_indices: vec![device_index],
            ..Config::default()
        };
        let whisper = Whisper::new(model_dir, config).map_err(|e| classify_load_error(&e))?;
        let mel = MelExtractor::new(whisper.n_mels());
        let vad = SileroVad::new()?;
        Ok(Self {
            whisper,
            tokenizer,
            special,
            mel,
            vad,
        })
    }

    /// Transcribes 16 kHz mono `audio`. The language is `opts.language` or detected on the first
    /// 30 s window. No speech gives an empty transcript whose language is the requested one (or
    /// empty when none was requested, since nothing was detected).
    pub fn transcribe(
        &mut self,
        audio: &[i16],
        opts: &TranscribeOptions,
    ) -> Result<Transcript, EngineError> {
        let Some(prepared) = prepare(&mut self.vad, &self.mel, audio, opts)? else {
            return Ok(transcript(
                Vec::new(),
                opts.language.clone().unwrap_or_default(),
            ));
        };
        let mut dec = Ct2Decoder {
            whisper: &self.whisper,
            tokenizer: &self.tokenizer,
            n_mels: self.whisper.n_mels(),
        };
        let language = match &opts.language {
            // faster-whisper forces English on English-only models.
            Some(_) | None if !self.whisper.is_multilingual() => "en".to_owned(),
            Some(lang) => lang.clone(),
            None => {
                let mut window = prepared.mel.window(0, N_FRAMES);
                detect_language(&dec, &mut window)?
            }
        };
        let lang_id = language_token(&self.tokenizer, &language)?;
        let segments = decode_prepared(
            &mut dec,
            &self.tokenizer,
            &self.special,
            lang_id,
            &prepared,
            opts,
        )?;
        Ok(transcript(segments, language))
    }

    /// Renders `vocab` as a decoder prompt of at most `max_tokens` tokens of this model.
    pub fn vocabulary_prompt(
        &self,
        vocab: &Vocabulary,
        prefix: &str,
        max_tokens: usize,
    ) -> Result<Option<String>, EngineError> {
        vocabulary_prompt(&self.tokenizer, vocab, prefix, max_tokens)
    }

    /// Runs one second of silence through the model without VAD so the first real request does
    /// not pay for CUDA kernel and allocator warm-up.
    pub fn warm_up(&mut self) -> Result<(), EngineError> {
        let opts = TranscribeOptions {
            vad: false,
            ..TranscribeOptions::default()
        };
        self.transcribe(&vec![0; crate::audio::SAMPLE_RATE], &opts)
            .map(|_| ())
    }
}

/// Top language of `window` as a code such as `tr`. faster-whisper uses the top result of the
/// first window (with one detection segment its threshold and majority vote pick the same one).
fn detect_language(dec: &Ct2Decoder<'_>, window: &mut [f32]) -> Result<String, EngineError> {
    let features = features(dec.n_mels, window)?;
    let results = dec.whisper.detect_language(&features).map_err(decode_err)?;
    let top = results
        .into_iter()
        .next()
        .and_then(|batch| batch.into_iter().next())
        .ok_or_else(|| EngineError::Decode("language detection returned nothing".to_owned()))?;
    // `<|tr|>` -> `tr`, like the reference's `token[2:-2]`.
    let token = top.language;
    token
        .strip_prefix("<|")
        .and_then(|t| t.strip_suffix("|>"))
        .map(str::to_owned)
        .ok_or_else(|| EngineError::Decode("unexpected language token".to_owned()))
}

/// Maps a `Whisper::new` failure. ct2rs flattens C++ exceptions into an error message, so a CUDA
/// failure (no driver, no device, bad device index) is recognised by the `CUDA` it always names;
/// everything else is a model the runtime could not load.
fn classify_load_error(e: &impl std::fmt::Display) -> EngineError {
    let message = e.to_string();
    if message.contains("CUDA") || message.contains("cuda") {
        EngineError::CudaUnavailable
    } else {
        EngineError::ModelDirInvalid { reason: message }
    }
}

fn transcript(segments: Vec<hece_core::Segment>, language: String) -> Transcript {
    let text = segments
        .iter()
        .map(|s| s.text.as_str())
        .collect::<String>()
        .trim()
        .to_owned();
    Transcript {
        text,
        segments: Some(segments),
        language,
        provider: PROVIDER.to_owned(),
        low_confidence: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_errors_naming_cuda_are_cuda_unavailable() {
        let cuda =
            classify_load_error(&"CUDA failed with error no CUDA-capable device is detected");
        assert_eq!(cuda.code(), "cuda_unavailable");
        let other = classify_load_error(&"Unable to open file 'model.bin'");
        assert_eq!(other.code(), "model_dir_invalid");
    }
}
