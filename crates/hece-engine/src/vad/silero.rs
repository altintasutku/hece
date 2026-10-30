//! Silero VAD v6 through ONNX Runtime.
//!
//! Ported from faster-whisper 1.2.1 (MIT), `vad.py` (`SileroVADModel`).

use ort::session::Session;
use ort::value::Tensor;

use super::{SpeechSpan, VadOptions, WINDOW, speech_timestamps};
use crate::error::EngineError;

/// Samples of the previous window prepended to each window.
const CONTEXT: usize = 64;
/// Silero hidden/cell state size.
const STATE: usize = 128;

static MODEL: &[u8] = include_bytes!("../../assets/silero_vad_v6.onnx");

fn vad_err(e: impl std::fmt::Display) -> EngineError {
    EngineError::Vad(e.to_string())
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
    ///
    /// Each call builds its own ONNX Runtime environment (cheap, reference counted); the VAD is
    /// created once per engine, so no process-wide state is needed.
    pub fn new() -> Result<Self, EngineError> {
        let env = ort::init().build().map_err(vad_err)?;
        let session = Session::builder(&env)
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

        let state = || Tensor::from_array(([1_usize, 1, STATE], vec![0.0_f32; STATE]));
        let outputs = self
            .session
            .run(ort::inputs! {
                "input" => Tensor::from_array(([rows, WINDOW + CONTEXT], batch)).map_err(vad_err)?,
                "h" => state().map_err(vad_err)?,
                "c" => state().map_err(vad_err)?,
            })
            .map_err(vad_err)?;
        let (_, probs) = outputs["speech_probs"]
            .try_extract_tensor::<f32>()
            .map_err(vad_err)?;
        Ok(probs.to_vec())
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
mod tests {
    use super::*;

    #[test]
    fn silero_vad_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<SileroVad>();
    }
}
