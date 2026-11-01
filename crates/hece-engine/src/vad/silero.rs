//! Silero VAD v6 through ONNX Runtime.
//!
//! Ported from faster-whisper 1.2.1 (MIT), `vad.py` (`SileroVADModel`).

use std::sync::Mutex;

use ort::environment::Environment;
use ort::ep;
use ort::session::Session;
use ort::value::Tensor;

use super::{SpeechSpan, VadOptions, WINDOW, speech_timestamps};
use crate::error::EngineError;

/// Samples of the previous window prepended to each window.
const CONTEXT: usize = 64;
/// Windows per model call (faster-whisper's `encoder_batch_size`).
const ENCODER_BATCH: usize = 10_000;
/// Silero hidden/cell state size.
const STATE: usize = 128;

static MODEL: &[u8] = include_bytes!("../../assets/silero_vad_v6.onnx");

fn vad_err(e: impl std::fmt::Display) -> EngineError {
    EngineError::Vad(e.to_string())
}

/// The process-wide ONNX Runtime environment. `ort` allows building only one per process (a
/// second `build` fails even after the first is dropped), so every VAD shares this one.
fn environment() -> Result<Environment, EngineError> {
    static ENV: Mutex<Option<Environment>> = Mutex::new(None);
    let mut env = ENV
        .lock()
        .map_err(|_| vad_err("onnx runtime environment lock poisoned"))?;
    if let Some(env) = env.as_ref() {
        return Ok(env.clone());
    }
    let built = ort::init().build().map_err(vad_err)?;
    *env = Some(built.clone());
    Ok(built)
}

/// The Silero VAD model. `Send`, so it can live in a worker thread; inference needs `&mut self`.
pub struct SileroVad {
    session: Session,
}

impl std::fmt::Debug for SileroVad {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SileroVad").finish_non_exhaustive()
    }
}

impl SileroVad {
    /// Loads the embedded model with one intra- and one inter-op thread.
    pub fn new() -> Result<Self, EngineError> {
        let env = environment()?;
        let session = Session::builder(&env)
            .map_err(vad_err)?
            // faster-whisper sets `enable_cpu_mem_arena = False`.
            .with_execution_providers([ep::CPU::default().with_arena_allocator(false).build()])
            .map_err(vad_err)?
            .with_intra_threads(1)
            .map_err(vad_err)?
            .with_inter_threads(1)
            .map_err(vad_err)?
            .commit_from_memory(MODEL)
            .map_err(vad_err)?;
        Ok(Self { session })
    }

    /// Speech probability per 512-sample window. Audio is zero padded to a whole number of
    /// windows (an exact multiple adds none). All windows run as one batch with zero `h`/`c`,
    /// each window preceded by the last 64 samples of the previous one, as faster-whisper does.
    pub fn speech_probs(&mut self, audio: &[f32]) -> Result<Vec<f32>, EngineError> {
        self.speech_probs_batched(audio, ENCODER_BATCH)
    }

    /// [`Self::speech_probs`] with an explicit number of windows per model call. The recurrent
    /// state returned by one call feeds the next, like faster-whisper's `encoder_batch_size`.
    fn speech_probs_batched(
        &mut self,
        audio: &[f32],
        batch_windows: usize,
    ) -> Result<Vec<f32>, EngineError> {
        let rows = audio.len().div_ceil(WINDOW);
        if rows == 0 {
            return Ok(Vec::new());
        }
        let mut padded = audio.to_vec();
        padded.resize(rows * WINDOW, 0.0);
        // faster-whisper zeroes the last 64 samples of the final window as a side effect of
        // building the context view in place; reproduce it for numerical parity.
        let tail = padded.len() - CONTEXT;
        padded[tail..].fill(0.0);

        let mut batch = Vec::with_capacity(rows * (WINDOW + CONTEXT));
        for r in 0..rows {
            if r == 0 {
                batch.extend(std::iter::repeat_n(0.0_f32, CONTEXT));
            } else {
                batch.extend_from_slice(&padded[r * WINDOW - CONTEXT..r * WINDOW]);
            }
            batch.extend_from_slice(&padded[r * WINDOW..(r + 1) * WINDOW]);
        }

        let mut h = vec![0.0_f32; STATE];
        let mut c = vec![0.0_f32; STATE];
        let mut probs = Vec::with_capacity(rows);
        for chunk in batch.chunks(batch_windows * (WINDOW + CONTEXT)) {
            let n = chunk.len() / (WINDOW + CONTEXT);
            let state = |v: &[f32]| Tensor::from_array(([1_usize, 1, STATE], v.to_vec()));
            let outputs = self
                .session
                .run(ort::inputs! {
                    "input" => Tensor::from_array(([n, WINDOW + CONTEXT], chunk.to_vec())).map_err(vad_err)?,
                    "h" => state(&h).map_err(vad_err)?,
                    "c" => state(&c).map_err(vad_err)?,
                })
                .map_err(vad_err)?;
            let (_, p) = outputs["speech_probs"]
                .try_extract_tensor::<f32>()
                .map_err(vad_err)?;
            probs.extend_from_slice(p);
            // The recurrent state is carried into the next batch.
            let (_, hn) = outputs["hn"].try_extract_tensor::<f32>().map_err(vad_err)?;
            let (_, cn) = outputs["cn"].try_extract_tensor::<f32>().map_err(vad_err)?;
            h = hn.to_vec();
            c = cn.to_vec();
        }
        Ok(probs)
    }

    /// Speech spans of `audio`: probabilities from [`Self::speech_probs`] fed to
    /// [`speech_timestamps`]. Like faster-whisper, the audio is padded by `512 - len % 512`
    /// zeros first (a full extra window when the length is already a multiple of 512).
    pub fn speech_spans(
        &mut self,
        audio: &[f32],
        opts: &VadOptions,
    ) -> Result<Vec<SpeechSpan>, EngineError> {
        if audio.is_empty() {
            return Ok(Vec::new());
        }
        let mut padded = audio.to_vec();
        padded.resize(audio.len() + WINDOW - audio.len() % WINDOW, 0.0);
        let probs = self.speech_probs(&padded)?;
        Ok(speech_timestamps(&probs, audio.len(), opts))
    }
}

#[cfg(test)]
#[expect(clippy::cast_possible_truncation, reason = "fixture decoding in tests")]
mod tests {
    use super::*;

    #[test]
    fn several_vads_can_exist_in_one_process() {
        let first = SileroVad::new().unwrap();
        let mut second = SileroVad::new().unwrap();
        drop(first);
        let mut third = SileroVad::new().unwrap();
        assert!(second.speech_probs(&[0.0; 512]).is_ok());
        assert!(third.speech_probs(&[0.0; 512]).is_ok());
    }

    #[test]
    fn silero_vad_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<SileroVad>();
    }

    #[test]
    fn small_batches_carry_state_and_match_faster_whisper() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/vad_probs.json")).unwrap();
        let audio: Vec<f32> = fixture["audio"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| f32::from(i16::try_from(v.as_i64().unwrap()).unwrap()) / 32768.0)
            .collect();
        let batch = usize::try_from(fixture["small_batch"].as_u64().unwrap()).unwrap();
        let expected: Vec<f32> = fixture["probs_small_batch"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_f64().unwrap() as f32)
            .collect();
        assert!(
            expected.len() > batch * 2,
            "fixture must span several batches"
        );

        let probs = SileroVad::new()
            .unwrap()
            .speech_probs_batched(&audio, batch)
            .unwrap();

        assert_eq!(probs.len(), expected.len());
        let max_diff = probs
            .iter()
            .zip(&expected)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0_f32, f32::max);
        assert!(max_diff < 1e-4, "max abs diff {max_diff}");
    }
}
