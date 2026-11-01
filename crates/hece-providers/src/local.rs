//! `local-ct2` adapter: the on-demand engine worker. The vocabulary goes to the worker as is;
//! it renders the prompt with the real Whisper tokenizer.

use std::sync::Arc;

use hece_core::ipc::MAX_AUDIO_SAMPLES;
use hece_core::{MAX_PROMPT_TOKENS, Transcript};

use crate::engine_client::EngineClient;
use crate::provider::{
    BoxFuture, Capabilities, Provider, ProviderError, ProviderRequest, VocabularyStyle,
};

/// Default provider name of the local engine.
pub const LOCAL_NAME: &str = "local";

const CAPS: Capabilities = Capabilities {
    local: true,
    vocabulary: VocabularyStyle::WhisperPrompt {
        max_tokens: MAX_PROMPT_TOKENS,
    },
    segments: true,
    max_audio_samples: MAX_AUDIO_SAMPLES,
    language_detection: true,
};

/// Local Whisper through the engine worker.
#[derive(Debug)]
pub struct LocalProvider {
    name: String,
    client: Arc<EngineClient>,
}

impl LocalProvider {
    /// Wraps an engine client; it may be shared with prewarm and shutdown callers.
    #[must_use]
    pub fn new(name: &str, client: Arc<EngineClient>) -> Self {
        Self {
            name: name.to_owned(),
            client,
        }
    }

    async fn run(&self, req: ProviderRequest<'_>) -> Result<Transcript, ProviderError> {
        if req.audio.len() > CAPS.max_audio_samples {
            return Err(ProviderError::AudioTooLong {
                provider: self.name.clone(),
            });
        }
        let mut transcript = self
            .client
            .transcribe(
                req.audio,
                req.language,
                req.vocabulary,
                req.prompt_prefix,
                req.prompt_tokens,
            )
            .await?;
        transcript.provider.clone_from(&self.name);
        Ok(transcript)
    }
}

impl Provider for LocalProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn capabilities(&self) -> &Capabilities {
        &CAPS
    }

    fn transcribe<'a>(
        &'a self,
        req: ProviderRequest<'a>,
    ) -> BoxFuture<'a, Result<Transcript, ProviderError>> {
        Box::pin(self.run(req))
    }
}
