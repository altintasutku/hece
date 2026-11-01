//! `hece dictate`: one recording from the microphone, stopped with Enter, transcribed and
//! printed. Also builds the provider shared with `hece run`.

use std::sync::Arc;

use anyhow::Context as _;
use hece_audio::{Capture, CaptureConfig, CaptureEvent};
use hece_core::Vocabulary;
use hece_providers::engine_client::{EngineClient, EngineConfig, ProcessSpawner};
use hece_providers::{
    KeySource, KeyringStore, LocalProvider, OpenAiConfig, OpenAiProvider, Provider, ProviderRequest,
};
use tokio::sync::mpsc;

use crate::cli::{DictateArgs, ProviderArgs};
use crate::output;

/// The configured provider, plus the engine client when it is the local one (for prewarm).
pub struct Setup {
    pub provider: Arc<dyn Provider>,
    pub engine: Option<Arc<EngineClient>>,
    pub vocabulary: Option<Vocabulary>,
}

pub fn setup(args: &ProviderArgs) -> anyhow::Result<Setup> {
    let vocabulary = match &args.vocab {
        Some(path) => Some(Vocabulary::parse(
            &std::fs::read_to_string(path)
                .with_context(|| format!("cannot read {}", path.display()))?,
        )),
        None => None,
    };
    if let Some(base) = &args.api_base {
        let mut cfg = OpenAiConfig::new("openai", base, &args.api_model);
        cfg.segments = args.segments;
        let key = args
            .key_env
            .clone()
            .map(|var| Arc::new(KeySource::new("openai", Some(var), Arc::new(KeyringStore))));
        return Ok(Setup {
            provider: Arc::new(OpenAiProvider::new(cfg, key)?),
            engine: None,
            vocabulary,
        });
    }
    let model = args
        .model
        .clone()
        .context("either --model (local engine) or --api-base is required")?;
    let mut cfg = EngineConfig::default_for(model)?;
    cfg.engine_path.clone_from(&args.engine);
    let engine = Arc::new(EngineClient::new(cfg, Arc::new(ProcessSpawner)));
    Ok(Setup {
        provider: Arc::new(LocalProvider::new("local", Arc::clone(&engine))),
        engine: Some(engine),
        vocabulary,
    })
}

pub async fn execute(args: &DictateArgs) -> anyhow::Result<()> {
    let setup = setup(&args.provider)?;
    let cfg = CaptureConfig {
        device: args.device.clone(),
        max_duration: std::time::Duration::from_secs(args.max_secs),
        ..CaptureConfig::default()
    };
    let (tx, mut events) = mpsc::unbounded_channel();
    let capture = Capture::start(&cfg, move |e| {
        let _ = tx.send(e);
    })?;
    output::status("recording; press Enter to stop");
    let mut enter = tokio::task::spawn_blocking(|| {
        let mut line = String::new();
        std::io::stdin().read_line(&mut line).map(|_| ())
    });
    loop {
        tokio::select! {
            _ = &mut enter => break,
            Some(event) = events.recv() => match event {
                CaptureEvent::Live { latency } => {
                    output::status(&format!("live after {} ms", latency.as_millis()));
                }
                CaptureEvent::LimitReached => {
                    output::status("recording limit reached");
                    break;
                }
                CaptureEvent::StreamError { code } => anyhow::bail!("microphone failed ({code})"),
            },
        }
    }
    let recording = tokio::task::spawn_blocking(move || capture.stop()).await??;
    output::status("transcribing");
    let req = ProviderRequest {
        audio: &recording.samples,
        language: args.provider.language.as_deref(),
        vocabulary: setup.vocabulary.as_ref(),
        prompt_prefix: hece_core::DEFAULT_PROMPT_PREFIX,
        prompt_tokens: hece_core::DEFAULT_PROMPT_TOKENS,
    };
    let transcript = setup
        .provider
        .transcribe(req)
        .await
        .map_err(|e| anyhow::anyhow!("{e} ({})", e.code()))?
        .with_quality_layers();
    if transcript.low_confidence {
        output::status("may be misheard");
    }
    output::line(&transcript.text);
    Ok(())
}

pub fn devices() -> anyhow::Result<()> {
    for name in hece_audio::input_devices()? {
        output::line(&name);
    }
    Ok(())
}
