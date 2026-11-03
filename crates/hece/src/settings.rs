//! What the settings app needs from Hece: read and edit `config.toml` (keeping the user's
//! comments), keep API keys in the OS keyring, and ask the running daemon to reload.
//! Values cross the boundary as JSON, keyed by dotted paths such as `provider.kind`.

use std::path::{Path, PathBuf};

use hece_providers::keys::{ApiKey, KeyringStore, SecretStore};
use serde_json::Value;
use toml_edit::{DocumentMut, Item, Table};

use crate::config::{Config, ConfigError};
use crate::vocabulary;

/// The config file, created from the commented template when missing.
pub fn config_file() -> anyhow::Result<PathBuf> {
    let path = crate::paths::config_file()?;
    match Config::load(&path) {
        Ok(_) | Err(ConfigError::Created(_) | ConfigError::Invalid { .. }) => Ok(path),
        Err(e) => Err(e.into()),
    }
}

/// The config file's current text, parsed for editing.
pub fn read(path: &Path) -> anyhow::Result<DocumentMut> {
    Ok(std::fs::read_to_string(path)?.parse()?)
}

/// The value at `key` (`section.field`), or `null` when it is not set.
#[must_use]
pub fn get(doc: &DocumentMut, key: &str) -> Value {
    let Some((section, field)) = key.split_once('.') else {
        return Value::Null;
    };
    let Some(item) = doc.get(section).and_then(|s| s.get(field)) else {
        return Value::Null;
    };
    match item.as_value() {
        Some(toml_edit::Value::String(s)) => Value::from(s.value().as_str()),
        Some(toml_edit::Value::Integer(i)) => Value::from(*i.value()),
        Some(toml_edit::Value::Boolean(b)) => Value::from(*b.value()),
        _ => Value::Null,
    }
}

/// Whether a config section exists (`fallback` and `cleanup` are optional).
#[must_use]
pub fn has_section(doc: &DocumentMut, section: &str) -> bool {
    doc.get(section).is_some_and(Item::is_table)
}

/// Applies `changes` (dotted key -> string, integer or bool; `null` removes the key) in one
/// write, unless they introduce a config error. Returns the validation message otherwise.
pub fn set(path: &Path, changes: &[(&str, Value)]) -> Result<(), String> {
    let mut doc = read(path).map_err(|e| e.to_string())?;
    for (key, value) in changes {
        let (section, field) = key
            .split_once('.')
            .ok_or_else(|| format!("bad key {key}"))?;
        let table = doc
            .entry(section)
            .or_insert_with(|| Item::Table(Table::new()))
            .as_table_mut()
            .ok_or_else(|| format!("[{section}] is not a table"))?;
        match value {
            Value::Null => {
                table.remove(field);
            }
            Value::String(s) => table[field] = toml_edit::value(s.as_str()),
            Value::Bool(b) => table[field] = toml_edit::value(*b),
            Value::Number(n) => {
                table[field] = toml_edit::value(n.as_i64().ok_or("numbers must be whole")?);
            }
            _ => return Err(format!("unsupported value for {key}")),
        }
    }
    write(path, &doc)
}

/// Removes an optional section (`fallback`, `cleanup`).
pub fn remove_section(path: &Path, section: &str) -> Result<(), String> {
    let mut doc = read(path).map_err(|e| e.to_string())?;
    doc.remove(section);
    write(path, &doc)
}

/// Writes `doc` unless it has a config error the file did not already have: one setting
/// that is still wrong (say, a model folder not prepared yet) must not block all the others.
fn write(path: &Path, doc: &DocumentMut) -> Result<(), String> {
    let text = doc.to_string();
    if let Err(e) = Config::parse(&text) {
        let before = std::fs::read_to_string(path).map(|old| Config::parse(&old).err());
        if before.ok().flatten().as_ref() != Some(&e) {
            return Err(e);
        }
    }
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, &text)
        .and_then(|()| std::fs::rename(&tmp, path))
        .map_err(|e| format!("could not save the config ({:?})", e.kind()))
}

/// The keyring entry name of a section's provider (`provider`, `fallback`, `cleanup`):
/// `name`, else the kind, as the daemon looks it up.
fn account(config: &Config, section: &str) -> Option<String> {
    match section {
        "provider" => Some(config.provider.name()),
        "fallback" => config
            .fallback
            .as_ref()
            .map(crate::config::ProviderConfig::name),
        "cleanup" => config
            .cleanup
            .as_ref()
            .map(crate::config::CleanupSection::name),
        _ => None,
    }
}

fn load(path: &Path) -> Result<Config, String> {
    Config::load(path).map_err(|e| e.to_string())
}

/// Whether a key is stored for the section's provider. Never returns the key.
pub fn key_stored(path: &Path, section: &str) -> Result<bool, String> {
    let account = account(&load(path)?, section).ok_or("no such provider")?;
    KeyringStore
        .get(&account)
        .map(|k| k.is_some())
        .map_err(|e| e.to_string())
}

/// Stores `secret` for the section's provider.
pub fn store_key(path: &Path, section: &str, secret: &str) -> Result<(), String> {
    let key = ApiKey::new(secret.trim());
    if key.expose().is_empty() {
        return Err("empty key".to_owned());
    }
    let account = account(&load(path)?, section).ok_or("no such provider")?;
    KeyringStore.set(&account, &key).map_err(|e| e.to_string())
}

/// Removes the key of the section's provider.
pub fn delete_key(path: &Path, section: &str) -> Result<(), String> {
    let account = account(&load(path)?, section).ok_or("no such provider")?;
    KeyringStore.delete(&account).map_err(|e| e.to_string())
}

/// Microphone names, as `[recording] device` expects them.
#[must_use]
pub fn devices() -> Vec<String> {
    hece_audio::input_devices().unwrap_or_default()
}

/// The vocabulary as the window edits it.
#[derive(Debug, serde::Serialize)]
pub struct VocabularyView {
    /// The file in use.
    pub path: String,
    /// Whether `vocabulary_file` names it (otherwise it is the default next to the config).
    pub custom: bool,
    /// Its terms in priority order, valid or not.
    pub entries: Vec<String>,
    /// How the entries are used.
    pub check: vocabulary::Check,
}

fn vocabulary_setup(config: &Path) -> Result<(PathBuf, bool, usize), String> {
    let doc = read(config).map_err(|e| e.to_string())?;
    let explicit = get(&doc, "transcription.vocabulary_file")
        .as_str()
        .map(PathBuf::from);
    let budget = get(&doc, "transcription.prompt_tokens")
        .as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .unwrap_or(hece_core::DEFAULT_PROMPT_TOKENS);
    let path = vocabulary::path(explicit.as_deref(), config);
    Ok((path, explicit.is_some(), budget))
}

/// The vocabulary file in use and its terms. Errors are stable codes.
pub fn vocabulary(config: &Path) -> Result<VocabularyView, String> {
    let (path, custom, budget) = vocabulary_setup(config)?;
    let text = vocabulary::read(&path).map_err(|e| e.code().to_owned())?;
    let entries = vocabulary::entries(&text);
    Ok(VocabularyView {
        path: path.display().to_string(),
        custom,
        check: vocabulary::check(&entries, budget),
        entries,
    })
}

/// Saves `entries` into the vocabulary file in use, keeping its comments.
pub fn save_vocabulary(config: &Path, entries: &[String]) -> Result<vocabulary::Check, String> {
    let (path, _, budget) = vocabulary_setup(config)?;
    vocabulary::save(&path, entries).map_err(|e| e.code().to_owned())?;
    Ok(vocabulary::check(entries, budget))
}

/// The terms of another file, to add to the list.
pub fn import_vocabulary(file: &Path) -> Result<Vec<String>, String> {
    std::fs::read_to_string(file)
        .map(|text| vocabulary::entries(&text))
        .map_err(|_| "vocabulary_unreadable".to_owned())
}

/// Makes `file` the vocabulary (`None`: back to the default next to the config).
pub fn use_vocabulary_file(config: &Path, file: Option<&Path>) -> Result<(), String> {
    let value = match file {
        Some(file) if !file.is_file() => return Err("vocabulary_unreadable".to_owned()),
        Some(file) => Value::from(file.display().to_string()),
        None => Value::Null,
    };
    set(config, &[("transcription.vocabulary_file", value)])
}

/// Asks the running daemon to re-read the config: `ok`, `busy`, `invalid_config`,
/// `not_running` or another stable code.
#[cfg(windows)]
pub async fn reload() -> String {
    let Ok(socket) = crate::paths::control_socket() else {
        return "no_state_dir".to_owned();
    };
    match hece_platform::control_request(&socket, "reload").await {
        Ok(answer) => answer,
        Err(e) => e.code().to_owned(),
    }
}

/// What the daemon is doing (`idle`, `recording`, `processing N`) or a stable error code.
#[cfg(windows)]
pub async fn status() -> String {
    let Ok(socket) = crate::paths::control_socket() else {
        return "no_state_dir".to_owned();
    };
    match hece_platform::control_request(&socket, "status").await {
        Ok(answer) => answer,
        Err(e) => e.code().to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_config(text: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("hece-settings-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{}.toml", text.len()));
        std::fs::write(&path, text).unwrap();
        path
    }

    #[test]
    fn edits_keep_comments_and_refuse_invalid_configs() {
        let path = temp_config(crate::config::TEMPLATE);
        set(&path, &[("recording.mode", Value::from("toggle"))]).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("# Hece configuration"), "comments kept");
        assert_eq!(get(&read(&path).unwrap(), "recording.mode"), "toggle");
        // max_seconds above the limit fails validation and leaves the file untouched.
        assert!(set(&path, &[("recording.max_seconds", Value::from(99_999))]).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        std::fs::remove_file(path).unwrap();
    }
}
