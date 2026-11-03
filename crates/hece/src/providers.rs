//! Builds providers from config sections (shared by `run` and `dictate`).

use std::sync::Arc;

use anyhow::Context as _;
use hece_core::Vocabulary;
use hece_providers::engine_client::{EngineClient, EngineConfig, ProcessSpawner};
use hece_providers::{
    Cleaner, CleanupApi, CleanupConfig, DeepgramConfig, DeepgramProvider, KeySource, KeyringStore,
    LocalProvider, OpenAiConfig, OpenAiProvider, Provider, Providers,
};

use crate::config::{CleanupKind, CleanupSection, Config, KeyRef, ProviderConfig, ProviderKind};

/// Everything a dictation needs besides the audio.
pub struct Setup {
    pub providers: Arc<Providers>,
    /// The local engine, if any provider uses it (for prewarm and shutdown).
    pub engine: Option<Arc<EngineClient>>,
    pub vocabulary: Option<Arc<Vocabulary>>,
    pub language: Option<String>,
    pub prompt_tokens: usize,
    /// The guarded LLM cleanup, when `[cleanup]` is configured.
    pub cleaner: Option<Arc<Cleaner>>,
}

fn key(p: &ProviderConfig) -> anyhow::Result<Option<Arc<KeySource>>> {
    Ok(key_source(
        &p.name(),
        p.key_ref().map_err(anyhow::Error::msg)?,
    ))
}

fn key_source(name: &str, key: KeyRef) -> Option<Arc<KeySource>> {
    match key {
        KeyRef::None => None,
        KeyRef::Keyring => Some(Arc::new(KeySource::new(name, None, Arc::new(KeyringStore)))),
        KeyRef::Env(var) => Some(Arc::new(KeySource::new(
            name,
            Some(var),
            Arc::new(KeyringStore),
        ))),
    }
}

/// One provider; `engine` is reused when both primary and fallback are local.
pub fn build(
    p: &ProviderConfig,
    engine: &mut Option<Arc<EngineClient>>,
) -> anyhow::Result<Arc<dyn Provider>> {
    let name = p.name();
    Ok(match p.kind {
        ProviderKind::Local => {
            let model = p.local_model().context("no local model")?;
            let client = if let Some(client) = engine {
                Arc::clone(client)
            } else {
                let mut cfg = EngineConfig::default_for(model)?;
                cfg.engine_path.clone_from(&p.engine);
                cfg.dll_dir = crate::cuda::engine_dll_dir()?;
                let client = Arc::new(EngineClient::new(cfg, Arc::new(ProcessSpawner)));
                *engine = Some(Arc::clone(&client));
                client
            };
            Arc::new(LocalProvider::new(&name, client))
        }
        ProviderKind::Openai => {
            let base = p.base_url.as_deref().context("base_url is required")?;
            let model = p.model.as_deref().context("model is required")?;
            let mut cfg = OpenAiConfig::new(&name, base, model);
            cfg.segments = p.segments;
            Arc::new(OpenAiProvider::new(cfg, key(p)?)?)
        }
        ProviderKind::Deepgram => {
            let mut cfg = DeepgramConfig::new(&name);
            if let Some(base) = &p.base_url {
                cfg.base_url.clone_from(base);
            }
            if let Some(model) = &p.model {
                cfg.model.clone_from(model);
            }
            let key = key(p)?.context("deepgram needs a key")?;
            Arc::new(DeepgramProvider::new(cfg, key)?)
        }
    })
}

pub fn read_vocabulary(cfg: &Config) -> anyhow::Result<Option<Arc<Vocabulary>>> {
    cfg.transcription
        .vocabulary_file
        .as_ref()
        .map(|path| {
            std::fs::read_to_string(path)
                .with_context(|| format!("cannot read {}", path.display()))
                .map(|text| Arc::new(Vocabulary::parse(&text)))
        })
        .transpose()
}

pub fn setup(cfg: &Config) -> anyhow::Result<Setup> {
    let mut engine = None;
    let primary = build(&cfg.provider, &mut engine)?;
    let fallback = cfg
        .fallback
        .as_ref()
        .map(|f| build(f, &mut engine))
        .transpose()?;
    let providers = Arc::new(Providers::new(
        primary,
        fallback,
        cfg.transcription.allow_cloud_fallback,
    )?);
    Ok(Setup {
        providers,
        engine,
        vocabulary: read_vocabulary(cfg)?,
        language: cfg.transcription.language.clone(),
        prompt_tokens: cfg.transcription.prompt_tokens,
        cleaner: cfg.cleanup.as_ref().map(cleaner).transpose()?,
    })
}

fn cleaner(c: &CleanupSection) -> anyhow::Result<Arc<Cleaner>> {
    let name = c.name();
    let (api, base, model) = match c.kind {
        CleanupKind::Anthropic => (
            CleanupApi::Anthropic,
            c.base_url
                .clone()
                .unwrap_or_else(|| hece_providers::cleanup::ANTHROPIC_BASE_URL.to_owned()),
            c.model
                .clone()
                .unwrap_or_else(|| hece_providers::cleanup::ANTHROPIC_MODEL.to_owned()),
        ),
        CleanupKind::Openai => (
            CleanupApi::OpenAiChat,
            c.base_url.clone().context("base_url is required")?,
            c.model.clone().context("model is required")?,
        ),
    };
    let key = key_source(&name, c.key_ref().map_err(anyhow::Error::msg)?);
    let cfg = CleanupConfig {
        name,
        api,
        base_url: base,
        model,
        timeout: std::time::Duration::from_secs(c.timeout_seconds),
    };
    Ok(Arc::new(Cleaner::new(cfg, key)?))
}
