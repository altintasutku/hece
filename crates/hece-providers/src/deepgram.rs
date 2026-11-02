//! `deepgram` adapter: pre-recorded `POST {base_url}/v1/listen` with the WAV as the body and
//! one `keyterm` query parameter per vocabulary term (Nova-3).

use std::sync::Arc;
use std::time::Duration;

use hece_core::Transcript;
use hece_core::ipc::MAX_AUDIO_SAMPLES;
use reqwest::Url;
use serde::Deserialize;

use crate::http;
use crate::keys::KeySource;
use crate::provider::{
    BoxFuture, Capabilities, Provider, ProviderError, ProviderRequest, VocabularyStyle,
};
use crate::vocab_hint::{VocabularyHint, render};

/// Deepgram's public API.
pub const DEFAULT_BASE_URL: &str = "https://api.deepgram.com";
/// Model with keyterm support.
pub const DEFAULT_MODEL: &str = "nova-3";
/// Deepgram recommends at most 20–50 key terms.
const MAX_KEYTERMS: usize = 50;
/// Deepgram rejects more than 500 tokens across all key terms.
const MAX_KEYTERM_TOKENS: usize = 500;

/// Settings of the Deepgram adapter.
#[derive(Debug, Clone)]
pub struct DeepgramConfig {
    /// Provider name; also the keyring account of its key.
    pub name: String,
    /// API base, [`DEFAULT_BASE_URL`] unless self-hosted or regional.
    pub base_url: String,
    /// Model, [`DEFAULT_MODEL`] by default.
    pub model: String,
    /// Request timeout before the per-audio-length allowance.
    pub timeout: Duration,
}

impl DeepgramConfig {
    /// Config for the public API with Nova-3 and a 60 s timeout.
    #[must_use]
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            base_url: DEFAULT_BASE_URL.to_owned(),
            model: DEFAULT_MODEL.to_owned(),
            timeout: Duration::from_mins(1),
        }
    }
}

/// The Deepgram pre-recorded API.
#[derive(Debug)]
pub struct DeepgramProvider {
    cfg: DeepgramConfig,
    url: Url,
    key: Arc<KeySource>,
    client: reqwest::Client,
    caps: Capabilities,
}

impl DeepgramProvider {
    /// Creates the adapter; Deepgram always needs a key.
    pub fn new(cfg: DeepgramConfig, key: Arc<KeySource>) -> Result<Self, ProviderError> {
        if cfg.model.trim().is_empty() {
            return Err(http::config(&cfg.name, "model is empty"));
        }
        let url = http::endpoint(&cfg.name, &cfg.base_url, "/v1/listen", true)?;
        let caps = Capabilities {
            local: http::is_loopback(&url),
            vocabulary: VocabularyStyle::Keyterms {
                max_terms: MAX_KEYTERMS,
                max_tokens: MAX_KEYTERM_TOKENS,
            },
            // Deepgram has no no-speech or log-probability signals for the confidence layer.
            segments: false,
            max_audio_samples: MAX_AUDIO_SAMPLES,
            language_detection: true,
        };
        Ok(Self {
            client: http::client(&cfg.name)?,
            cfg,
            url,
            key,
            caps,
        })
    }

    fn url_for(&self, req: &ProviderRequest<'_>) -> Url {
        let mut url = self.url.clone();
        {
            let mut q = url.query_pairs_mut();
            q.append_pair("model", &self.cfg.model)
                .append_pair("smart_format", "true");
            match http::requested_language(req.language) {
                Some(language) => q.append_pair("language", language),
                None => q.append_pair("detect_language", "true"),
            };
            // One parameter per term: commas inside a value are taken literally, silently.
            if let VocabularyHint::Keyterms(terms) =
                render(self.caps.vocabulary, req.vocabulary, "", 0)
            {
                for term in &terms {
                    q.append_pair("keyterm", term);
                }
            }
        }
        url
    }

    async fn run(&self, req: ProviderRequest<'_>) -> Result<Transcript, ProviderError> {
        let name = self.cfg.name.as_str();
        if req.audio.len() > self.caps.max_audio_samples {
            return Err(ProviderError::AudioTooLong {
                provider: name.to_owned(),
            });
        }
        let builder = self
            .client
            .post(self.url_for(&req))
            .timeout(http::request_timeout(self.cfg.timeout, req.audio.len()))
            .header(
                reqwest::header::AUTHORIZATION,
                format!("Token {}", self.key.get().await?.expose()),
            )
            .header(reqwest::header::CONTENT_TYPE, "audio/wav")
            .body(http::wav(name, req.audio)?);
        tracing::debug!(
            provider = name,
            samples = req.audio.len(),
            "sending transcription"
        );
        let body = match http::send(name, builder).await {
            Err(e @ ProviderError::Unauthorized { .. }) => {
                self.key.invalidate().await;
                return Err(e);
            }
            other => other?,
        };
        let parsed: Response = http::parse_json(name, &body)?;
        let channel = parsed
            .results
            .channels
            .into_iter()
            .next()
            .ok_or_else(|| http::bad_response(name))?;
        let text = channel
            .alternatives
            .into_iter()
            .next()
            .map(|a| a.transcript)
            .ok_or_else(|| http::bad_response(name))?;
        let language = http::requested_language(req.language)
            .map(str::to_owned)
            .or_else(|| http::language_code(channel.detected_language.as_deref()))
            .unwrap_or_default();
        Ok(Transcript {
            text: text.trim().to_owned(),
            segments: None,
            language,
            provider: name.to_owned(),
            low_confidence: false,
        })
    }
}

impl Provider for DeepgramProvider {
    fn name(&self) -> &str {
        &self.cfg.name
    }

    fn capabilities(&self) -> &Capabilities {
        &self.caps
    }

    fn transcribe<'a>(
        &'a self,
        req: ProviderRequest<'a>,
    ) -> BoxFuture<'a, Result<Transcript, ProviderError>> {
        Box::pin(self.run(req))
    }
}

#[derive(Deserialize)]
struct Response {
    results: Results,
}

#[derive(Deserialize)]
struct Results {
    channels: Vec<Channel>,
}

#[derive(Deserialize)]
struct Channel {
    alternatives: Vec<Alternative>,
    #[serde(default)]
    detected_language: Option<String>,
}

#[derive(Deserialize)]
struct Alternative {
    transcript: String,
}
