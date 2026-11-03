//! `openai-compatible` adapter: `POST {base_url}/audio/transcriptions` (multipart). Covers
//! `OpenAI`, Groq, speaches, `whisper.cpp` server and `LocalAI`.

use std::sync::Arc;
use std::time::Duration;

use hece_core::ipc::MAX_AUDIO_SAMPLES;
use hece_core::{Segment, Transcript};
use reqwest::Url;
use reqwest::multipart::{Form, Part};
use serde::Deserialize;

use crate::http;
use crate::keys::KeySource;
use crate::provider::{
    BoxFuture, Capabilities, Provider, ProviderError, ProviderRequest, VocabularyStyle,
};
use crate::vocab_hint::{VocabularyHint, render};

/// Whisper's prompt limit; the `OpenAI` docs give 224 tokens for `whisper-1`.
const PROMPT_TOKENS: usize = 224;

/// Settings of one OpenAI-compatible endpoint.
#[derive(Debug, Clone)]
pub struct OpenAiConfig {
    /// Provider name; also the keyring account of its key.
    pub name: String,
    /// API base including the version, e.g. `https://api.openai.com/v1`.
    pub base_url: String,
    /// Model, e.g. `whisper-1` or `whisper-large-v3-turbo`.
    pub model: String,
    /// Request `verbose_json` and use its segments. Only Whisper-based models support it.
    pub segments: bool,
    /// Request timeout before the per-audio-length allowance.
    pub timeout: Duration,
}

impl OpenAiConfig {
    /// Config with `segments` off and a 60 s timeout.
    #[must_use]
    pub fn new(name: &str, base_url: &str, model: &str) -> Self {
        Self {
            name: name.to_owned(),
            base_url: base_url.to_owned(),
            model: model.to_owned(),
            segments: false,
            timeout: Duration::from_mins(1),
        }
    }
}

/// An OpenAI-compatible transcription endpoint.
#[derive(Debug)]
pub struct OpenAiProvider {
    cfg: OpenAiConfig,
    url: Url,
    key: Option<Arc<KeySource>>,
    client: reqwest::Client,
    caps: Capabilities,
}

impl OpenAiProvider {
    /// Creates the adapter. `key` is `None` for local servers that need no authentication.
    pub fn new(cfg: OpenAiConfig, key: Option<Arc<KeySource>>) -> Result<Self, ProviderError> {
        if cfg.model.trim().is_empty() {
            return Err(http::config(&cfg.name, "model is empty"));
        }
        let url = http::endpoint(
            &cfg.name,
            &cfg.base_url,
            "/audio/transcriptions",
            key.is_some(),
        )?;
        let caps = Capabilities {
            local: http::is_loopback(&url),
            vocabulary: VocabularyStyle::WhisperPrompt {
                max_tokens: PROMPT_TOKENS,
            },
            segments: cfg.segments,
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

    async fn run(&self, req: ProviderRequest<'_>) -> Result<Transcript, ProviderError> {
        let name = self.cfg.name.as_str();
        let language = http::requested_language(req.language);
        if req.audio.len() > self.caps.max_audio_samples {
            return Err(ProviderError::AudioTooLong {
                provider: name.to_owned(),
            });
        }
        let file = Part::bytes(http::wav(name, req.audio)?)
            .file_name("audio.wav")
            .mime_str("audio/wav")
            .map_err(|_| http::config(name, "invalid mime type"))?;
        let format = if self.cfg.segments {
            "verbose_json"
        } else {
            "json"
        };
        let mut form = Form::new()
            .part("file", file)
            .text("model", self.cfg.model.clone())
            .text("response_format", format);
        if let Some(language) = language {
            form = form.text("language", language.to_owned());
        }
        if let VocabularyHint::Prompt(prompt) = render(
            self.caps.vocabulary,
            req.vocabulary,
            req.prompt_prefix,
            req.prompt_tokens,
        ) {
            form = form.text("prompt", prompt);
        }
        let mut builder = self
            .client
            .post(self.url.clone())
            .timeout(http::request_timeout(self.cfg.timeout, req.audio.len()))
            .multipart(form);
        if let Some(key) = &self.key {
            builder = builder.bearer_auth(key.get().await?.expose());
        }
        tracing::debug!(
            provider = name,
            samples = req.audio.len(),
            "sending transcription"
        );
        let body = match http::send(name, builder).await {
            Err(e @ ProviderError::Unauthorized { .. }) => {
                if let Some(key) = &self.key {
                    key.invalidate().await;
                }
                return Err(e);
            }
            other => other?,
        };
        let parsed: Response = http::parse_json(name, &body)?;
        Ok(parsed.into_transcript(name, language, self.cfg.segments))
    }
}

impl Provider for OpenAiProvider {
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
    text: String,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    segments: Option<Vec<WireSegment>>,
}

/// A `verbose_json` segment; servers that omit the confidence fields count as confident.
#[derive(Deserialize)]
struct WireSegment {
    text: String,
    start: f32,
    end: f32,
    #[serde(default)]
    avg_logprob: f32,
    #[serde(default)]
    no_speech_prob: f32,
}

impl Response {
    fn into_transcript(
        self,
        provider: &str,
        requested: Option<&str>,
        segments: bool,
    ) -> Transcript {
        let language = requested
            .map(str::to_owned)
            .or_else(|| http::language_code(self.language.as_deref()))
            .unwrap_or_default();
        let segments = segments.then_some(self.segments).flatten().map(|segs| {
            segs.into_iter()
                .map(|s| Segment {
                    text: s.text,
                    start_s: s.start,
                    end_s: s.end,
                    no_speech_prob: s.no_speech_prob,
                    avg_logprob: s.avg_logprob,
                })
                .collect()
        });
        Transcript {
            text: self.text.trim().to_owned(),
            segments,
            language,
            provider: provider.to_owned(),
            low_confidence: false,
        }
    }
}
