//! `config.toml`: the source of truth for the daemon, validated at startup with errors that name
//! the field. A commented template is written on first run.

use std::path::{Path, PathBuf};

use hece_core::ipc::MAX_AUDIO_SAMPLES;
use serde::Deserialize;

/// Longest recording the providers accept.
const MAX_SECONDS: u64 = (MAX_AUDIO_SAMPLES / 16_000) as u64;

/// Written when no config exists yet.
pub const TEMPLATE: &str = r#"# Hece configuration. Restart hece after editing.

[recording]
mode = "hold"              # hold: hold Ctrl+Shift+Space while talking; toggle: press to start/stop
# device = "Microphone (...)" # a name from `hece devices`; default: the system microphone
max_seconds = 600          # at most 660

[transcription]
language = "tr"            # remove for auto-detect
# vocabulary_file = 'C:\Users\you\hece-vocabulary.txt'  # one term per line, # comments
prompt_tokens = 200
allow_cloud_fallback = false   # a local provider may fall back to a cloud one only if true

[provider]
kind = "local"             # local | openai | deepgram
model_dir = 'C:\path\to\large-v3-turkish-ec03ccf'      # scripts/prepare_model.py output
# engine = 'C:\path\to\hece-engine.exe'                 # default: next to hece.exe

# OpenAI-compatible example (OpenAI, Groq, speaches, whisper.cpp server):
# kind = "openai"
# name = "groq"            # shown in messages; the keyring entry is named after it
# base_url = "https://api.groq.com/openai/v1"
# model = "whisper-large-v3"
# key = "keyring"          # keyring (set with `hece key set groq`) | none | env:VARIABLE
# segments = true          # Whisper models only

# Optional fallback for a whole dictation, same fields as [provider]:
# [fallback]
# kind = "deepgram"
# key = "keyring"

# Optional LLM cleanup (fillers, punctuation, repeats; guarded so meaning never changes):
# [cleanup]
# kind = "anthropic"       # anthropic | openai (OpenAI, OpenRouter, Ollama, llama.cpp server)
# model = "claude-opus-5-5"
# key = "keyring"          # `hece key set anthropic`
# base_url = "http://127.0.0.1:11434/v1"   # openai kind only
# timeout_seconds = 10

[api]
enabled = false            # local OpenAI-compatible API on 127.0.0.1; token in %LOCALAPPDATA%\hece\api-token
port = 8765

[history]
enabled = true
max_entries = 50
max_days = 7
"#;

/// The whole file.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Config {
    pub recording: Recording,
    pub transcription: Transcription,
    pub provider: ProviderConfig,
    pub fallback: Option<ProviderConfig>,
    pub cleanup: Option<CleanupSection>,
    pub api: ApiConfig,
    pub history: HistoryConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RecordMode {
    #[default]
    Hold,
    Toggle,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Recording {
    pub mode: RecordMode,
    pub device: Option<String>,
    pub max_seconds: u64,
}

impl Default for Recording {
    fn default() -> Self {
        Self {
            mode: RecordMode::Hold,
            device: None,
            max_seconds: 600,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Transcription {
    pub language: Option<String>,
    pub vocabulary_file: Option<PathBuf>,
    pub prompt_tokens: usize,
    pub allow_cloud_fallback: bool,
}

impl Default for Transcription {
    fn default() -> Self {
        Self {
            language: None,
            vocabulary_file: None,
            prompt_tokens: hece_core::DEFAULT_PROMPT_TOKENS,
            allow_cloud_fallback: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    #[default]
    Local,
    Openai,
    Deepgram,
}

/// Where a provider's API key comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyRef {
    /// The keyring entry named after the provider.
    Keyring,
    /// No key (e.g. a local server).
    None,
    /// This environment variable.
    Env(String),
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ProviderConfig {
    pub kind: ProviderKind,
    pub name: Option<String>,
    pub model_dir: Option<PathBuf>,
    pub engine: Option<PathBuf>,
    pub base_url: Option<String>,
    pub model: Option<String>,
    pub key: Option<String>,
    pub segments: bool,
}

impl ProviderConfig {
    /// Display name and keyring account.
    pub fn name(&self) -> String {
        self.name.clone().unwrap_or_else(|| {
            match self.kind {
                ProviderKind::Local => "local",
                ProviderKind::Openai => "openai",
                ProviderKind::Deepgram => "deepgram",
            }
            .to_owned()
        })
    }

    /// The parsed `key` field; defaults to the keyring.
    pub fn key_ref(&self) -> Result<KeyRef, String> {
        match self.key.as_deref() {
            None | Some("keyring") => Ok(KeyRef::Keyring),
            Some("none") => Ok(KeyRef::None),
            Some(v) => match v.strip_prefix("env:") {
                Some(var) if !var.trim().is_empty() => Ok(KeyRef::Env(var.trim().to_owned())),
                // Never echo the value: it may be a pasted key.
                _ => Err(
                    "key must be keyring, none or env:VARIABLE (never the key itself)".to_owned(),
                ),
            },
        }
    }

    fn validate(&self, section: &str) -> Result<(), String> {
        let missing =
            |field: &str| format!("[{section}] {field} is required for kind = {:?}", self.kind);
        let key = self.key_ref().map_err(|e| format!("[{section}] {e}"))?;
        match self.kind {
            ProviderKind::Local => {
                let dir = self
                    .model_dir
                    .as_ref()
                    .ok_or_else(|| missing("model_dir"))?;
                if !dir.join("model.bin").is_file() {
                    return Err(format!(
                        "[{section}] model_dir {} has no model.bin (run scripts/prepare_model.py)",
                        dir.display()
                    ));
                }
            }
            ProviderKind::Openai => {
                self.base_url.as_ref().ok_or_else(|| missing("base_url"))?;
                self.model.as_ref().ok_or_else(|| missing("model"))?;
            }
            ProviderKind::Deepgram if key == KeyRef::None => {
                return Err(format!("[{section}] deepgram needs a key"));
            }
            ProviderKind::Deepgram => {}
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CleanupKind {
    #[default]
    Anthropic,
    Openai,
}

/// `[cleanup]`: present means enabled.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct CleanupSection {
    pub kind: CleanupKind,
    pub name: Option<String>,
    pub base_url: Option<String>,
    pub model: Option<String>,
    pub key: Option<String>,
    pub timeout_seconds: u64,
}

impl Default for CleanupSection {
    fn default() -> Self {
        Self {
            kind: CleanupKind::Anthropic,
            name: None,
            base_url: None,
            model: None,
            key: None,
            timeout_seconds: 10,
        }
    }
}

impl CleanupSection {
    /// Display name and keyring account.
    pub fn name(&self) -> String {
        self.name.clone().unwrap_or_else(|| {
            match self.kind {
                CleanupKind::Anthropic => "anthropic",
                CleanupKind::Openai => "cleanup",
            }
            .to_owned()
        })
    }

    /// The key source, parsed like `[provider] key`.
    pub fn key_ref(&self) -> Result<KeyRef, String> {
        ProviderConfig {
            key: self.key.clone(),
            ..ProviderConfig::default()
        }
        .key_ref()
        .map_err(|e| format!("[cleanup] {e}"))
    }

    fn validate(&self) -> Result<(), String> {
        self.key_ref()?;
        if !(1..=120).contains(&self.timeout_seconds) {
            return Err("[cleanup] timeout_seconds must be 1..=120".to_owned());
        }
        if self.kind == CleanupKind::Openai && (self.base_url.is_none() || self.model.is_none()) {
            return Err("[cleanup] kind = \"openai\" needs base_url and model".to_owned());
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ApiConfig {
    pub enabled: bool,
    pub port: u16,
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            port: 8765,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct HistoryConfig {
    pub enabled: bool,
    pub max_entries: usize,
    pub max_days: u64,
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_entries: 50,
            max_days: 7,
        }
    }
}

/// Config problems; messages name the file and the field.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("created {0}; edit it (at least [provider]) and start hece again")]
    Created(PathBuf),
    #[error("cannot read {path}: {kind:?}")]
    Unreadable {
        path: PathBuf,
        kind: std::io::ErrorKind,
    },
    #[error("{path}: {message}")]
    Invalid { path: PathBuf, message: String },
}

impl Config {
    /// Parses and validates `text`.
    pub fn parse(text: &str) -> Result<Self, String> {
        // Only the message and line: the toml error's Display quotes the offending line, which
        // may hold a pasted key, and this text reaches the log and a toast.
        let cfg: Self = toml::from_str(text).map_err(|e| match e.span() {
            Some(span) => format!(
                "line {}: {}",
                text[..span.start].matches('\n').count() + 1,
                e.message()
            ),
            None => e.message().to_owned(),
        })?;
        cfg.validate()?;
        Ok(cfg)
    }

    fn validate(&self) -> Result<(), String> {
        if !(1..=MAX_SECONDS).contains(&self.recording.max_seconds) {
            return Err(format!("[recording] max_seconds must be 1..={MAX_SECONDS}"));
        }
        if self.transcription.prompt_tokens > hece_core::MAX_PROMPT_TOKENS {
            return Err(format!(
                "[transcription] prompt_tokens must be at most {}",
                hece_core::MAX_PROMPT_TOKENS
            ));
        }
        if self.history.enabled && self.history.max_entries == 0 {
            return Err("[history] max_entries must be at least 1 (or enabled = false)".to_owned());
        }
        self.provider.validate("provider")?;
        if let Some(cleanup) = &self.cleanup {
            cleanup.validate()?;
        }
        if let Some(fallback) = &self.fallback {
            fallback.validate("fallback")?;
        }
        Ok(())
    }

    /// Loads `path`; writes the template and reports [`ConfigError::Created`] when missing.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if let Some(dir) = path.parent() {
                    std::fs::create_dir_all(dir).map_err(|e| ConfigError::Unreadable {
                        path: dir.to_owned(),
                        kind: e.kind(),
                    })?;
                }
                std::fs::write(path, TEMPLATE).map_err(|e| ConfigError::Unreadable {
                    path: path.to_owned(),
                    kind: e.kind(),
                })?;
                return Err(ConfigError::Created(path.to_owned()));
            }
            Err(e) => {
                return Err(ConfigError::Unreadable {
                    path: path.to_owned(),
                    kind: e.kind(),
                });
            }
        };
        Self::parse(&text).map_err(|message| ConfigError::Invalid {
            path: path.to_owned(),
            message,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_parses_up_to_the_model_check() {
        let err = Config::parse(TEMPLATE).unwrap_err();
        assert!(err.contains("model.bin"), "{err}");
    }

    #[test]
    fn errors_name_the_field() {
        let err = Config::parse("[recording]\nmode = \"hold\"\nmax_secs = 3\n").unwrap_err();
        assert!(err.contains("max_secs"), "{err}");
        let err = Config::parse("[provider]\nkind = \"openai\"\nmodel = \"m\"\n").unwrap_err();
        assert_eq!(err, "[provider] base_url is required for kind = Openai");
        let err = Config::parse("[provider]\nkind = \"deepgram\"\nkey = \"none\"\n").unwrap_err();
        assert_eq!(err, "[provider] deepgram needs a key");
        // A pasted key in a malformed or misnamed line must not be quoted back.
        for text in [
            "[provider]\nkey = sk-pasted\n",
            "[provider]\napi_key = \"sk-pasted\"\n",
        ] {
            let err = Config::parse(text).unwrap_err();
            assert!(
                err.starts_with("line 2:") && !err.contains("sk-pasted"),
                "{err}"
            );
        }
    }

    #[test]
    fn key_refs() {
        let p = |key: &str| ProviderConfig {
            key: Some(key.to_owned()),
            ..ProviderConfig::default()
        };
        assert_eq!(
            p("env:GROQ_KEY").key_ref(),
            Ok(KeyRef::Env("GROQ_KEY".into()))
        );
        assert_eq!(p("none").key_ref(), Ok(KeyRef::None));
        assert!(p("plain-secret").key_ref().is_err());
    }
}
