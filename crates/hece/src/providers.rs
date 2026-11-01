//! Builds providers from config sections (shared by `run` and `dictate`).

use std::sync::Arc;

use anyhow::Context as _;
use hece_core::Vocabulary;
use hece_providers::engine_client::{EngineClient, EngineConfig, ProcessSpawner};
use hece_providers::{
    DeepgramConfig, DeepgramProvider, KeySource, KeyringStore, LocalProvider, OpenAiConfig,
    OpenAiProvider, Provider, Providers,
};

use crate::config::{Config, KeyRef, ProviderConfig, ProviderKind};

/// Everything a dictation needs besides the audio.
pub struct Setup {
    pub providers: Providers,
    /// The local engine, if any provider uses it (for prewarm and shutdown).
    pub engine: Option<Arc<EngineClient>>,
    pub vocabulary: Option<Arc<Vocabulary>>,
    pub language: Option<String>,
    pub prompt_tokens: usize,
}

fn key(p: &ProviderConfig) -> anyhow::Result<Option<Arc<KeySource>>> {
    let name = p.name();
    Ok(match p.key_ref().map_err(anyhow::Error::msg)? {
        KeyRef::None => None,
        KeyRef::Keyring => Some(Arc::new(KeySource::new(
            &name,
            None,
            Arc::new(KeyringStore),
        ))),
        KeyRef::Env(var) => Some(Arc::new(KeySource::new(
            &name,
            Some(var),
            Arc::new(KeyringStore),
        ))),
    })
}

/// One provider; `engine` is reused when both primary and fallback are local.
pub fn build(
    p: &ProviderConfig,
    engine: &mut Option<Arc<EngineClient>>,
) -> anyhow::Result<Arc<dyn Provider>> {
    let name = p.name();
    Ok(match p.kind {
        ProviderKind::Local => {
            let model = p.model_dir.clone().context("model_dir is required")?;
            let client = if let Some(client) = engine {
                Arc::clone(client)
            } else {
                let mut cfg = EngineConfig::default_for(model)?;
                cfg.engine_path.clone_from(&p.engine);
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
    let providers = Providers::new(primary, fallback, cfg.transcription.allow_cloud_fallback)?;
    Ok(Setup {
        providers,
        engine,
        vocabulary: read_vocabulary(cfg)?,
        language: cfg.transcription.language.clone(),
        prompt_tokens: cfg.transcription.prompt_tokens,
    })
}
