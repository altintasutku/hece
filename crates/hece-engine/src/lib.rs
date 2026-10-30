//! Local speech-to-text engine: mel spectrogram, VAD, `CTranslate2` decode and the worker
//! IPC server.

pub mod audio;
pub mod decode;
pub mod error;
pub mod mel;
pub mod tokens;
pub mod vad;
