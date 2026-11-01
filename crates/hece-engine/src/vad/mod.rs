//! Voice activity detection: Silero VAD probabilities and the speech-timestamp state machine.
//!
//! Ported from faster-whisper 1.2.1 (MIT), `vad.py`.

mod silero;
mod timestamps;

pub use silero::SileroVad;
pub use timestamps::{
    SpeechSpan, SpeechTimestampsMap, VadOptions, collect_chunks, speech_timestamps,
};

/// Samples per Silero window.
pub(crate) const WINDOW: usize = 512;
