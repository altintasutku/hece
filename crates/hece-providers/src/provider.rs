//! The [`Provider`] trait, its declared [`Capabilities`] and the shared [`ProviderError`].

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use hece_core::{Transcript, Vocabulary};

use crate::engine_client::EngineClientError;

/// A boxed, sendable future; keeps [`Provider`] usable as `dyn Provider`.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// How a provider accepts the custom vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VocabularyStyle {
    /// The vocabulary is ignored.
    Unsupported,
    /// Rendered into a Whisper-style text prompt of at most `max_tokens` tokens.
    WhisperPrompt {
        /// Token limit of the prompt.
        max_tokens: usize,
    },
    /// Sent as separate key terms.
    Keyterms {
        /// Maximum number of terms per request.
        max_terms: usize,
        /// Maximum tokens across all terms.
        max_tokens: usize,
    },
}

/// What an adapter supports, decided per adapter (including "not supported").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    /// Audio never leaves the machine.
    pub local: bool,
    /// How the vocabulary is passed.
    pub vocabulary: VocabularyStyle,
    /// Results carry segments with confidence signals.
    pub segments: bool,
    /// Longest accepted audio, in 16 kHz mono samples.
    pub max_audio_samples: usize,
    /// Detects the language when none is requested.
    pub language_detection: bool,
}

/// One whole dictation to transcribe.
#[derive(Clone, Copy)]
pub struct ProviderRequest<'a> {
    /// 16 kHz mono samples.
    pub audio: &'a [i16],
    /// Language code, or `None` to auto-detect.
    pub language: Option<&'a str>,
    /// Custom vocabulary, rendered per [`Capabilities::vocabulary`].
    pub vocabulary: Option<&'a Vocabulary>,
    /// Text placed before the term list in a prompt.
    pub prompt_prefix: &'a str,
    /// Token budget for a vocabulary prompt.
    pub prompt_tokens: usize,
}

impl std::fmt::Debug for ProviderRequest<'_> {
    /// Redacted: audio, vocabulary and prompt are personal data, so only sizes show.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderRequest")
            .field("samples", &self.audio.len())
            .field("language", &self.language)
            .field("vocabulary_terms", &self.vocabulary.map(|v| v.terms.len()))
            .field("prompt_tokens", &self.prompt_tokens)
            .finish_non_exhaustive()
    }
}

/// A speech-to-text backend.
pub trait Provider: Send + Sync + std::fmt::Debug {
    /// Configured name, shown to the user and stored in [`Transcript::provider`].
    fn name(&self) -> &str;

    /// What this adapter supports.
    fn capabilities(&self) -> &Capabilities;

    /// Transcribes one whole dictation.
    fn transcribe<'a>(
        &'a self,
        req: ProviderRequest<'a>,
    ) -> BoxFuture<'a, Result<Transcript, ProviderError>>;
}

/// Failures of any provider. [`ProviderError::code`] is the stable user-facing code; messages
/// name the provider but never carry keys, audio or transcripts.
#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    /// The provider configuration is invalid.
    #[error("{provider}: invalid configuration: {reason}")]
    Config {
        /// Provider name.
        provider: String,
        /// What is wrong.
        reason: String,
    },
    /// No API key in the named env var or the keyring.
    #[error("{provider}: no API key is set")]
    KeyMissing {
        /// Provider name.
        provider: String,
    },
    /// The keyring is locked or refused access.
    #[error("{provider}: the keyring is locked; unlock it to use this provider")]
    KeyringLocked {
        /// Provider name.
        provider: String,
    },
    /// The keyring failed for another reason.
    #[error("{provider}: keyring error: {kind}")]
    Keyring {
        /// Provider name.
        provider: String,
        /// Failure kind, without secret material.
        kind: String,
    },
    /// The provider rejected the API key (401/403).
    #[error("{provider}: the API key was rejected")]
    Unauthorized {
        /// Provider name.
        provider: String,
    },
    /// The provider is rate limiting (429).
    #[error("{provider}: rate limited")]
    RateLimited {
        /// Provider name.
        provider: String,
        /// Wait time the provider asked for, if any.
        retry_after: Option<Duration>,
    },
    /// No answer in time.
    #[error("{provider}: request timed out")]
    Timeout {
        /// Provider name.
        provider: String,
    },
    /// The provider could not be reached.
    #[error("{provider}: network error")]
    Network {
        /// Provider name.
        provider: String,
    },
    /// The provider failed (5xx).
    #[error("{provider}: server error {status}")]
    Server {
        /// Provider name.
        provider: String,
        /// HTTP status.
        status: u16,
    },
    /// The provider refused the request (other 4xx).
    #[error("{provider}: request rejected with status {status}")]
    Rejected {
        /// Provider name.
        provider: String,
        /// HTTP status.
        status: u16,
    },
    /// The response could not be understood.
    #[error("{provider}: unexpected response")]
    BadResponse {
        /// Provider name.
        provider: String,
    },
    /// The audio is longer than the provider accepts.
    #[error("{provider}: audio is too long")]
    AudioTooLong {
        /// Provider name.
        provider: String,
    },
    /// The local engine failed.
    #[error(transparent)]
    Engine(#[from] EngineClientError),
    /// Both the primary and the fallback provider failed.
    #[error("{primary}; fallback: {fallback}")]
    AllFailed {
        /// Error of the primary provider.
        primary: Box<ProviderError>,
        /// Error of the fallback provider.
        fallback: Box<ProviderError>,
    },
}

impl ProviderError {
    /// Stable error code safe to show to users.
    #[must_use]
    pub fn code(&self) -> &str {
        match self {
            Self::Config { .. } => "provider_config",
            Self::KeyMissing { .. } => "provider_key_missing",
            Self::KeyringLocked { .. } => "keyring_locked",
            Self::Keyring { .. } => "keyring_error",
            Self::Unauthorized { .. } => "provider_unauthorized",
            Self::RateLimited { .. } => "provider_rate_limited",
            Self::Timeout { .. } => "provider_timeout",
            Self::Network { .. } => "provider_network",
            Self::Server { .. } => "provider_server_error",
            Self::Rejected { .. } => "provider_rejected",
            Self::BadResponse { .. } => "provider_bad_response",
            Self::AudioTooLong { .. } => "audio_too_long",
            Self::Engine(e) => e.code(),
            Self::AllFailed { .. } => "all_providers_failed",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_errors_keep_their_code() {
        let e = ProviderError::from(EngineClientError::Timeout);
        assert_eq!(e.code(), "engine_timeout");
    }

    #[test]
    fn messages_name_the_provider() {
        let e = ProviderError::KeyringLocked {
            provider: "groq".to_owned(),
        };
        assert!(e.to_string().starts_with("groq:"));
        assert_eq!(e.code(), "keyring_locked");
    }

    #[test]
    fn request_debug_hides_content() {
        let vocab = Vocabulary {
            terms: vec!["Gizli Proje".to_owned()],
        };
        let req = ProviderRequest {
            audio: &[1, 2, 3],
            language: Some("tr"),
            vocabulary: Some(&vocab),
            prompt_prefix: "Vocabulary: ",
            prompt_tokens: 200,
        };
        let shown = format!("{req:?}");
        assert!(!shown.contains("Gizli"));
        assert!(shown.contains("samples: 3"));
    }
}
