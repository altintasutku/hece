//! `hece dictate`: one recording from the microphone, stopped with Enter, transcribed and
//! printed. Provider flags are mapped onto the same config sections `run` uses.

use hece_audio::{Capture, CaptureConfig, CaptureEvent};
use tokio::sync::mpsc;

use crate::cli::{DictateArgs, ProviderArgs};
use crate::config::{Config, ProviderConfig, ProviderKind};
use crate::output;
use crate::providers::setup;

fn config(args: &ProviderArgs) -> Config {
    let mut cfg = Config::default();
    cfg.transcription.language.clone_from(&args.language);
    cfg.transcription.vocabulary_file.clone_from(&args.vocab);
    cfg.provider = if args.api_base.is_some() {
        ProviderConfig {
            kind: ProviderKind::Openai,
            base_url: args.api_base.clone(),
            model: Some(args.api_model.clone()),
            key: Some(
                args.key_env
                    .as_ref()
                    .map_or_else(|| "none".to_owned(), |v| format!("env:{v}")),
            ),
            segments: args.segments,
            ..ProviderConfig::default()
        }
    } else {
        ProviderConfig {
            kind: ProviderKind::Local,
            model_dir: args.model.clone(),
            engine: args.engine.clone(),
            ..ProviderConfig::default()
        }
    };
    cfg
}

pub async fn execute(args: &DictateArgs) -> anyhow::Result<()> {
    if args.provider.api_base.is_none() && args.provider.model.is_none() {
        anyhow::bail!("either --model (local engine) or --api-base is required");
    }
    let setup = setup(&config(&args.provider))?;
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
    let params = crate::longform::Params {
        language: setup.language.clone(),
        vocabulary: setup.vocabulary.clone(),
        prompt_tokens: setup.prompt_tokens,
    };
    let outcome = crate::longform::transcribe(&setup.providers, &recording.samples, &params)
        .await
        .map_err(|e| anyhow::anyhow!("{e} ({})", e.code()))?;
    let transcript = outcome.transcript.with_quality_layers();
    if transcript.low_confidence {
        output::status("may be misheard");
    }
    output::line(&transcript.text);
    if let Some(engine) = setup.engine {
        let _ = engine.shutdown().await;
    }
    Ok(())
}

pub fn devices() -> anyhow::Result<()> {
    for name in hece_audio::input_devices()? {
        output::line(&name);
    }
    Ok(())
}
