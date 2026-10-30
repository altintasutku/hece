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
    /// The requested language is not a language of this model's vocabulary.
    #[error("unsupported language")]
    UnsupportedLanguage,
    /// `XDG_RUNTIME_DIR` is not set, so there is no place for the worker socket.
    #[error("XDG_RUNTIME_DIR is not set")]
    NoRuntimeDir,
    /// A socket operation of the worker failed.
    #[error("socket failure: {0}")]
    Socket(String),
    /// The input audio cannot be processed. Reserved for request validation by the worker.
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
            Self::UnsupportedLanguage => "unsupported_language",
            Self::InvalidAudio { .. } => "invalid_audio",
            Self::NoRuntimeDir => "no_runtime_dir",
            Self::Socket(_) => "socket_failed",
        }
    }
}

/// Substrings of the `CTranslate2` exceptions raised when the CUDA runtime or device is unusable:
/// `CUDA_CHECK` (no device, driver too old, invalid device index, out of memory), `CUBLAS_CHECK`,
/// a build without CUDA (`devices.cc`) and a failed `dlopen` of cuBLAS (`cublas_stub.cc`).
/// Case-sensitive, so a model path containing "cuda" does not match.
const CUDA_ERRORS: [&str; 4] = [
    "CUDA failed with error ",
    "cuBLAS failed with status ",
    "This CTranslate2 package was not compiled with CUDA support",
    " is not found or cannot be loaded",
];

fn is_cuda_error(message: &str) -> bool {
    CUDA_ERRORS.iter().any(|m| message.contains(m))
}

/// Maps a model-load failure. `ct2rs` flattens C++ exceptions into an error message, so CUDA
/// failures are recognised by [`CUDA_ERRORS`]. Everything else is `ModelDirInvalid`, including
/// a compute type the device cannot run ("Requested ... compute type, but the target device or
/// backend do not support ..."): the converted model and the requested precision do not fit.
#[cfg_attr(
    all(not(test), not(feature = "cuda")),
    expect(dead_code, reason = "used by the cuda backend")
)]
pub(crate) fn classify_load_error(e: &impl std::fmt::Display) -> EngineError {
    let message = e.to_string();
    if is_cuda_error(&message) {
        EngineError::CudaUnavailable
    } else {
        EngineError::ModelDirInvalid { reason: message }
    }
}

/// Maps an encode, generate or language-detection failure: CUDA failures that surface at first
/// use (out of memory, a missing cuBLAS library) are `CudaUnavailable`, the rest `Decode`.
#[cfg_attr(
    all(not(test), not(feature = "cuda")),
    expect(dead_code, reason = "used by the cuda backend")
)]
pub(crate) fn classify_runtime_error(e: &impl std::fmt::Display) -> EngineError {
    let message = e.to_string();
    if is_cuda_error(&message) {
        EngineError::CudaUnavailable
    } else {
        EngineError::Decode(message)
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
            (EngineError::UnsupportedLanguage, "unsupported_language"),
            (
                EngineError::InvalidAudio { reason: "x".into() },
                "invalid_audio",
            ),
            (EngineError::NoRuntimeDir, "no_runtime_dir"),
            (EngineError::Socket("x".into()), "socket_failed"),
        ];
        for (err, code) in cases {
            assert_eq!(err.code(), code);
        }
    }

    #[test]
    fn ctranslate2_cuda_errors_are_cuda_unavailable() {
        for message in [
            "CUDA failed with error no CUDA-capable device is detected",
            "CUDA failed with error CUDA driver version is insufficient for CUDA runtime version",
            "CUDA failed with error invalid device ordinal",
            "cuBLAS failed with status CUBLAS_STATUS_NOT_INITIALIZED",
            "This CTranslate2 package was not compiled with CUDA support",
            "CUDA failed with error out of memory",
            "Library libcublas.so.12 is not found or cannot be loaded",
        ] {
            assert_eq!(classify_load_error(&message).code(), "cuda_unavailable");
            assert_eq!(classify_runtime_error(&message).code(), "cuda_unavailable");
        }
    }

    #[test]
    fn plain_decode_errors_stay_decode_failed() {
        let err = classify_runtime_error(&"inconsistent tensor shapes");
        assert_eq!(err.code(), "decode_failed");
    }

    #[test]
    fn other_load_errors_are_model_dir_invalid() {
        for message in [
            "Unable to open file 'model.bin' in model '/models/cuda-whisper/'",
            "Unable to open file '/models/CUDA-whisper/model.bin'",
            "Requested int8_float16 compute type, but the target device or backend do not \
             support efficient int8_float16 computation.",
        ] {
            assert_eq!(classify_load_error(&message).code(), "model_dir_invalid");
        }
    }
}
