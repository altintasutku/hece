//! Wire types and framing for the engine worker socket. No I/O: callers do their own reads and
//! writes and use these helpers for the codec.

use std::path::PathBuf;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::types::Transcript;
use crate::vocab::Vocabulary;

/// Maximum size of one frame body in bytes.
pub const MAX_FRAME_BYTES: usize = 64 * 1024 * 1024;
/// Maximum audio length per request: 11 minutes of 16 kHz mono samples.
pub const MAX_AUDIO_SAMPLES: usize = 16_000 * 60 * 11;
/// Protocol version announced in [`Response::Hello`].
pub const PROTOCOL_VERSION: u32 = 1;
/// How long the worker keeps a connection open without a frame. Clients should reconnect
/// before an idle connection reaches this age.
pub const CONN_IDLE_TIMEOUT: Duration = Duration::from_secs(30);
/// Compute type used when a request does not name one.
pub const DEFAULT_COMPUTE: &str = "int8_float16";

fn default_compute() -> String {
    DEFAULT_COMPUTE.to_owned()
}

/// Errors from the IPC codec.
#[derive(Debug, thiserror::Error)]
pub enum IpcError {
    /// A frame is larger than [`MAX_FRAME_BYTES`].
    #[error("frame of {0} bytes exceeds the limit")]
    FrameTooLarge(usize),
    /// The frame body is not valid JSON for the expected type.
    #[error("invalid json: {0}")]
    Json(String),
    /// The audio payload is not valid base64.
    #[error("invalid base64 audio")]
    Base64,
    /// The decoded audio has an odd number of bytes.
    #[error("audio byte length is odd")]
    OddAudioLength,
    /// The audio holds more than [`MAX_AUDIO_SAMPLES`] samples.
    #[error("audio too long: more than {0} samples")]
    AudioTooLong(usize),
}

impl IpcError {
    /// Stable error code safe to show to users.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::FrameTooLarge(_) => "frame_too_large",
            Self::Json(_) => "bad_json",
            Self::Base64 | Self::OddAudioLength => "bad_audio",
            Self::AudioTooLong(_) => "invalid_audio",
        }
    }
}

/// Audio as base64 of little-endian `i16` samples (16 kHz mono).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AudioB64(pub String);

impl std::fmt::Debug for AudioB64 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "AudioB64({} chars)", self.0.len())
    }
}

impl AudioB64 {
    /// Encodes samples as little-endian bytes in standard base64.
    #[must_use]
    pub fn encode(samples: &[i16]) -> Self {
        let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        Self(STANDARD.encode(bytes))
    }

    /// Decodes back to samples, rejecting odd byte counts and audio over [`MAX_AUDIO_SAMPLES`].
    pub fn decode(&self) -> Result<Vec<i16>, IpcError> {
        // Reject before decoding so an oversized payload never allocates.
        let max_encoded = (MAX_AUDIO_SAMPLES * 2).div_ceil(3) * 4;
        if self.0.len() > max_encoded {
            return Err(IpcError::AudioTooLong(MAX_AUDIO_SAMPLES));
        }
        let bytes = STANDARD.decode(&self.0).map_err(|_| IpcError::Base64)?;
        if bytes.len() % 2 != 0 {
            return Err(IpcError::OddAudioLength);
        }
        if bytes.len() / 2 > MAX_AUDIO_SAMPLES {
            return Err(IpcError::AudioTooLong(MAX_AUDIO_SAMPLES));
        }
        Ok(bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| i16::from_le_bytes(*c))
            .collect())
    }
}

/// Parameters for preloading a model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoadRequest {
    /// Directory holding the `CTranslate2` model.
    pub model_dir: PathBuf,
    /// Compute type, e.g. `int8_float16`.
    #[serde(default = "default_compute")]
    pub compute: String,
}

/// Parameters for one transcription.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct TranscribeRequest {
    /// Directory holding the `CTranslate2` model.
    pub model_dir: PathBuf,
    /// Compute type, e.g. `int8_float16`.
    #[serde(default = "default_compute")]
    pub compute: String,
    /// Language code, or `None` to auto-detect.
    pub language: Option<String>,
    /// Custom vocabulary rendered into the Whisper prompt.
    pub vocabulary: Option<Vocabulary>,
    /// Text placed before the term list in the prompt.
    pub prompt_prefix: String,
    /// Token budget for the vocabulary prompt.
    pub prompt_tokens: usize,
    /// Whether to run voice activity detection.
    pub vad: bool,
    /// The audio to transcribe.
    pub audio: AudioB64,
}

impl std::fmt::Debug for TranscribeRequest {
    /// Redacted: vocabulary terms, prompt text and audio are personal data, so only sizes show.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TranscribeRequest")
            .field("model_dir", &self.model_dir)
            .field("compute", &self.compute)
            .field("language", &self.language)
            .field(
                "vocabulary_terms",
                &self.vocabulary.as_ref().map(|v| v.terms.len()),
            )
            .field("prompt_prefix_chars", &self.prompt_prefix.chars().count())
            .field("prompt_tokens", &self.prompt_tokens)
            .field("vad", &self.vad)
            .field("audio", &self.audio)
            .finish()
    }
}

/// Client-to-worker message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    /// Handshake.
    Hello,
    /// Preload a model.
    Load(LoadRequest),
    /// Transcribe audio.
    Transcribe(TranscribeRequest),
    /// Ask the worker to exit.
    Shutdown,
}

/// Worker-to-client message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Response {
    /// Handshake reply.
    Hello {
        /// Protocol version of the worker.
        protocol: u32,
        /// Engine crate version.
        engine_version: String,
        /// Model currently loaded, if any.
        loaded_model: Option<PathBuf>,
    },
    /// A model finished loading.
    Loaded {
        /// Load time in milliseconds.
        load_ms: u64,
    },
    /// Transcription result.
    Transcript(Transcript),
    /// The request failed; only a stable code crosses the socket.
    Error {
        /// Stable error code.
        code: String,
    },
    /// The worker is exiting.
    Bye,
}

/// Serializes `msg` into a frame: 4-byte big-endian length followed by the JSON body.
pub fn encode_frame<T: Serialize>(msg: &T) -> Result<Vec<u8>, IpcError> {
    let body = serde_json::to_vec(msg).map_err(|e| IpcError::Json(e.to_string()))?;
    if body.len() > MAX_FRAME_BYTES {
        return Err(IpcError::FrameTooLarge(body.len()));
    }
    let len = u32::try_from(body.len()).map_err(|_| IpcError::FrameTooLarge(body.len()))?;
    let mut frame = Vec::with_capacity(4 + body.len());
    frame.extend_from_slice(&len.to_be_bytes());
    frame.extend_from_slice(&body);
    Ok(frame)
}

/// Reads the body length from a frame header, rejecting oversized frames.
pub fn frame_len(header: [u8; 4]) -> Result<usize, IpcError> {
    let len = u32::from_be_bytes(header) as usize;
    if len > MAX_FRAME_BYTES {
        return Err(IpcError::FrameTooLarge(len));
    }
    Ok(len)
}

/// Parses a frame body.
pub fn decode_body<T: DeserializeOwned>(body: &[u8]) -> Result<T, IpcError> {
    serde_json::from_slice(body).map_err(|e| IpcError::Json(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip<T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug>(msg: &T) {
        let frame = encode_frame(msg).unwrap();
        let header: [u8; 4] = frame[..4].try_into().unwrap();
        let len = frame_len(header).unwrap();
        assert_eq!(len, frame.len() - 4);
        let back: T = decode_body(&frame[4..]).unwrap();
        assert_eq!(&back, msg);
    }

    fn transcribe_request() -> TranscribeRequest {
        TranscribeRequest {
            model_dir: PathBuf::from("/models/m"),
            compute: DEFAULT_COMPUTE.to_owned(),
            language: Some("en".to_owned()),
            vocabulary: Some(Vocabulary::parse("alpha\nbeta")),
            prompt_prefix: "Vocabulary: ".to_owned(),
            prompt_tokens: 64,
            vad: true,
            audio: AudioB64::encode(&[1, 2, 3]),
        }
    }

    #[test]
    fn request_variants_roundtrip() {
        roundtrip(&Request::Hello);
        roundtrip(&Request::Load(LoadRequest {
            model_dir: PathBuf::from("/models/m"),
            compute: "float16".to_owned(),
        }));
        roundtrip(&Request::Transcribe(transcribe_request()));
        roundtrip(&Request::Shutdown);
    }

    #[test]
    fn response_variants_roundtrip() {
        roundtrip(&Response::Hello {
            protocol: PROTOCOL_VERSION,
            engine_version: "0.1.0".to_owned(),
            loaded_model: Some(PathBuf::from("/models/m")),
        });
        roundtrip(&Response::Loaded { load_ms: 11_000 });
        roundtrip(&Response::Transcript(Transcript {
            text: "hello".to_owned(),
            segments: None,
            language: "en".to_owned(),
            provider: "local".to_owned(),
            low_confidence: false,
        }));
        roundtrip(&Response::Error {
            code: "bad_audio".to_owned(),
        });
        roundtrip(&Response::Bye);
    }

    #[test]
    fn debug_output_is_redacted() {
        let mut req = transcribe_request();
        req.prompt_prefix = "secretprefix".to_owned();
        let text = format!("{req:?} {:?}", Request::Transcribe(req.clone()));
        for leak in ["alpha", "beta", "secretprefix"] {
            assert!(!text.contains(leak), "{text}");
        }
        assert!(text.contains("vocabulary_terms: Some(2)"), "{text}");
    }

    #[test]
    fn hello_json_shape() {
        let json = serde_json::to_string(&Request::Hello).unwrap();
        assert_eq!(json, r#"{"type":"hello"}"#);
    }

    #[test]
    fn unknown_tag_is_rejected_but_unknown_fields_are_allowed() {
        let err = decode_body::<Request>(br#"{"type":"nope"}"#).unwrap_err();
        assert_eq!(err.code(), "bad_json");
        let ok: Request = decode_body(br#"{"type":"load","model_dir":"/m","extra":1}"#).unwrap();
        match ok {
            Request::Load(l) => assert_eq!(l.compute, DEFAULT_COMPUTE),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn audio_roundtrip() {
        let samples = [i16::MIN, -1, 0, 1, i16::MAX];
        assert_eq!(AudioB64::encode(&samples).decode().unwrap(), samples);
    }

    #[test]
    fn audio_odd_length_rejected() {
        let audio = AudioB64(STANDARD.encode([1u8, 2, 3]));
        assert!(matches!(audio.decode(), Err(IpcError::OddAudioLength)));
    }

    #[test]
    fn audio_too_long_rejected() {
        let audio = AudioB64::encode(&vec![0i16; MAX_AUDIO_SAMPLES + 1]);
        assert!(matches!(audio.decode(), Err(IpcError::AudioTooLong(_))));
        let at_limit = AudioB64::encode(&vec![0i16; MAX_AUDIO_SAMPLES]);
        assert_eq!(at_limit.decode().unwrap().len(), MAX_AUDIO_SAMPLES);
    }

    #[test]
    fn audio_invalid_base64_rejected() {
        let audio = AudioB64("not base64!!".to_owned());
        assert!(matches!(audio.decode(), Err(IpcError::Base64)));
    }

    #[test]
    fn oversized_frame_header_rejected() {
        let header = u32::try_from(MAX_FRAME_BYTES + 1).unwrap().to_be_bytes();
        assert!(matches!(frame_len(header), Err(IpcError::FrameTooLarge(_))));
        let ok = u32::try_from(MAX_FRAME_BYTES).unwrap().to_be_bytes();
        assert_eq!(frame_len(ok).unwrap(), MAX_FRAME_BYTES);
    }

    #[test]
    fn error_codes() {
        assert_eq!(IpcError::FrameTooLarge(1).code(), "frame_too_large");
        assert_eq!(IpcError::Json(String::new()).code(), "bad_json");
        assert_eq!(IpcError::Base64.code(), "bad_audio");
        assert_eq!(IpcError::OddAudioLength.code(), "bad_audio");
        assert_eq!(IpcError::AudioTooLong(1).code(), "invalid_audio");
    }
}
