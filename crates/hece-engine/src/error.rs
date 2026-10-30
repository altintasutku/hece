//! Engine error type with stable machine-readable codes.

use thiserror::Error;

/// Errors produced by the local engine. Inner strings come from library errors, never from
/// audio or transcript text.
#[derive(Debug, Error)]
pub enum EngineError {
    /// The model directory is missing required files or is unreadable.
    #[error("invalid model directory: {reason}")]
    ModelDirInvalid {
        /// What is wrong with the directory.
        reason: String,
    },
    /// No usable CUDA device was found.
    #[error("CUDA is not available on this machine")]
    CudaUnavailable,
    /// The binary was built without the `cuda` feature.
    #[error("this build does not include CUDA support")]
    BuiltWithoutCuda,
    /// The voice activity detector failed.
    #[error("voice activity detection failed: {0}")]
    Vad(String),
    /// The tokenizer failed to load or run.
    #[error("tokenizer failed: {0}")]
    Tokenizer(String),
    /// The decoder failed.
    #[error("decoding failed: {0}")]
    Decode(String),
    /// The input audio cannot be processed.
    #[error("invalid audio: {reason}")]
    InvalidAudio {
        /// Why the audio was rejected.
        reason: String,
    },
}

impl EngineError {
    /// Stable code shown to the user and used in logs.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::ModelDirInvalid { .. } => "model_dir_invalid",
            Self::CudaUnavailable => "cuda_unavailable",
            Self::BuiltWithoutCuda => "built_without_cuda",
            Self::Vad(_) => "vad_failed",
            Self::Tokenizer(_) => "tokenizer_failed",
            Self::Decode(_) => "decode_failed",
            Self::InvalidAudio { .. } => "invalid_audio",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_variant_has_its_code() {
        let cases = [
            (
                EngineError::ModelDirInvalid { reason: "x".into() },
                "model_dir_invalid",
            ),
            (EngineError::CudaUnavailable, "cuda_unavailable"),
            (EngineError::BuiltWithoutCuda, "built_without_cuda"),
            (EngineError::Vad("x".into()), "vad_failed"),
            (EngineError::Tokenizer("x".into()), "tokenizer_failed"),
            (EngineError::Decode("x".into()), "decode_failed"),
            (
                EngineError::InvalidAudio { reason: "x".into() },
                "invalid_audio",
            ),
        ];
        for (err, code) in cases {
            assert_eq!(err.code(), code);
        }
    }
}
