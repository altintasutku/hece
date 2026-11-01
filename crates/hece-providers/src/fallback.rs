//! Fallback for a whole dictation. It never applies per chunk: one dictation is transcribed by
//! one provider, so a result never mixes two models. Sending audio from a local primary to a
//! cloud fallback needs an explicit opt-in, and every fallback is reported in [`Outcome`].

use std::sync::Arc;

use hece_core::Transcript;

use crate::provider::{Provider, ProviderError, ProviderRequest};

/// Why and to whom a dictation fell back; the user is always told.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FallbackNotice {
    /// Provider that failed.
    pub from: String,
    /// Provider that answered.
    pub to: String,
    /// Stable error code of the failure.
    pub reason: String,
}

/// A transcript and, if the fallback answered, why.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    /// The transcript.
    pub transcript: Transcript,
    /// Set when the fallback provider produced the transcript.
    pub fell_back: Option<FallbackNotice>,
}

/// A primary provider with an optional fallback.
#[derive(Debug, Clone)]
pub struct Providers {
    primary: Arc<dyn Provider>,
    fallback: Option<Arc<dyn Provider>>,
}

impl Providers {
    /// Pairs `primary` with `fallback`. A local primary with a cloud fallback is refused unless
    /// `allow_local_to_cloud` is set, because it would send audio off the machine.
    pub fn new(
        primary: Arc<dyn Provider>,
        fallback: Option<Arc<dyn Provider>>,
        allow_local_to_cloud: bool,
    ) -> Result<Self, ProviderError> {
        if let Some(fb) = &fallback {
            if fb.name() == primary.name() {
                return Err(ProviderError::Config {
                    provider: fb.name().to_owned(),
                    reason: "fallback is the same provider as the primary".to_owned(),
                });
            }
            if primary.capabilities().local && !fb.capabilities().local && !allow_local_to_cloud {
                return Err(ProviderError::Config {
                    provider: fb.name().to_owned(),
                    reason: "a cloud fallback for a local provider must be allowed explicitly"
                        .to_owned(),
                });
            }
        }
        Ok(Self { primary, fallback })
    }

    /// The primary provider.
    #[must_use]
    pub fn primary(&self) -> &Arc<dyn Provider> {
        &self.primary
    }

    /// Transcribes the whole dictation with the primary, and on any failure with the fallback.
    pub async fn transcribe(&self, req: ProviderRequest<'_>) -> Result<Outcome, ProviderError> {
        let primary_err = match self.primary.transcribe(req).await {
            Ok(transcript) => {
                return Ok(Outcome {
                    transcript,
                    fell_back: None,
                });
            }
            Err(e) => e,
        };
        let Some(fallback) = &self.fallback else {
            return Err(primary_err);
        };
        tracing::warn!(
            from = self.primary.name(),
            to = fallback.name(),
            code = primary_err.code(),
            "primary provider failed, using fallback"
        );
        match fallback.transcribe(req).await {
            Ok(transcript) => Ok(Outcome {
                transcript,
                fell_back: Some(FallbackNotice {
                    from: self.primary.name().to_owned(),
                    to: fallback.name().to_owned(),
                    reason: primary_err.code().to_owned(),
                }),
            }),
            Err(fallback_err) => Err(ProviderError::AllFailed {
                primary: Box::new(primary_err),
                fallback: Box::new(fallback_err),
            }),
        }
    }
}
