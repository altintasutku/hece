//! Whole-dictation fallback: when it runs, what the user is told, and the local → cloud opt-in.
#![allow(clippy::unwrap_used, reason = "tests may unwrap")]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use hece_core::Transcript;
use hece_core::ipc::MAX_AUDIO_SAMPLES;
use hece_providers::{
    BoxFuture, Capabilities, Provider, ProviderError, ProviderRequest, Providers, VocabularyStyle,
};

#[derive(Debug)]
struct Fake {
    name: &'static str,
    caps: Capabilities,
    fails: bool,
    calls: AtomicUsize,
}

fn fake(name: &'static str, local: bool, fails: bool) -> Arc<Fake> {
    Arc::new(Fake {
        name,
        caps: Capabilities {
            local,
            vocabulary: VocabularyStyle::Unsupported,
            segments: false,
            max_audio_samples: MAX_AUDIO_SAMPLES,
            language_detection: true,
        },
        fails,
        calls: AtomicUsize::new(0),
    })
}

impl Provider for Fake {
    fn name(&self) -> &str {
        self.name
    }

    fn capabilities(&self) -> &Capabilities {
        &self.caps
    }

    fn transcribe<'a>(
        &'a self,
        req: ProviderRequest<'a>,
    ) -> BoxFuture<'a, Result<Transcript, ProviderError>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fails {
                return Err(ProviderError::Network {
                    provider: self.name.to_owned(),
                });
            }
            Ok(Transcript {
                text: format!("{} samples", req.audio.len()),
                segments: None,
                language: String::new(),
                provider: self.name.to_owned(),
                low_confidence: false,
            })
        })
    }
}

const AUDIO: [i16; 480] = [0; 480];

fn request() -> ProviderRequest<'static> {
    ProviderRequest {
        audio: &AUDIO,
        language: None,
        vocabulary: None,
        prompt_prefix: "",
        prompt_tokens: 0,
    }
}

#[tokio::test]
async fn primary_success_does_not_touch_the_fallback() {
    let primary = fake("openai", false, false);
    let fallback = fake("local", true, false);
    let providers = Providers::new(primary.clone(), Some(fallback.clone()), false).unwrap();
    let out = providers.transcribe(request()).await.unwrap();
    assert_eq!(out.transcript.provider, "openai");
    assert!(out.fell_back.is_none());
    assert_eq!(fallback.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn failure_retries_the_whole_dictation_and_tells_the_user() {
    let primary = fake("openai", false, true);
    let fallback = fake("local", true, false);
    let providers = Providers::new(primary, Some(fallback), false).unwrap();
    let out = providers.transcribe(request()).await.unwrap();
    assert_eq!(out.transcript.provider, "local");
    // The fallback got all the audio, not a remainder.
    assert_eq!(out.transcript.text, "480 samples");
    let notice = out.fell_back.unwrap();
    assert_eq!(notice.from, "openai");
    assert_eq!(notice.to, "local");
    assert_eq!(notice.reason, "provider_network");
}

#[tokio::test]
async fn both_failing_reports_both() {
    let providers = Providers::new(
        fake("openai", false, true),
        Some(fake("groq", false, true)),
        false,
    )
    .unwrap();
    let e = providers.transcribe(request()).await.unwrap_err();
    assert_eq!(e.code(), "all_providers_failed");
    let msg = e.to_string();
    assert!(msg.contains("openai") && msg.contains("groq"));
}

#[test]
fn local_to_cloud_needs_explicit_opt_in() {
    let e = Providers::new(
        fake("local", true, false),
        Some(fake("openai", false, false)),
        false,
    )
    .unwrap_err();
    assert_eq!(e.code(), "provider_config");
    assert!(
        Providers::new(
            fake("local", true, false),
            Some(fake("openai", false, false)),
            true
        )
        .is_ok()
    );
}

#[test]
fn fallback_cannot_be_the_primary() {
    let e = Providers::new(
        fake("openai", false, false),
        Some(fake("openai", false, false)),
        false,
    )
    .unwrap_err();
    assert_eq!(e.code(), "provider_config");
}
