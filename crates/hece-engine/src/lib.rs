//! Local speech-to-text engine: mel spectrogram, VAD, `CTranslate2` decode and the worker
//! IPC server.

pub mod audio;
#[cfg(feature = "cuda")]
mod ct2;
pub mod decode;
pub mod engine;
pub mod error;
pub mod mel;
pub mod tokens;
pub mod vad;

#[cfg(feature = "cuda")]
pub use ct2::LocalEngine;
pub use engine::{ComputeKind, TranscribeOptions, transcribe_with};
