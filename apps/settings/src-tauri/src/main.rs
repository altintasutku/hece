//! Hece settings window. Edits `config.toml` through `hece::settings` (comments kept, invalid
//! edits refused), keeps keys in the Credential Manager and asks the daemon to reload after
//! every change. Keys never travel back to the window; it only learns whether one is stored.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::process::ExitCode;

use hece::settings;
use serde_json::{Map, Value};

/// Every config value the window shows or edits.
const KEYS: &[&str] = &[
    "provider.kind",
    "provider.name",
    "provider.model_dir",
    "provider.base_url",
    "provider.model",
    "provider.key",
    "transcription.language",
    "transcription.allow_cloud_fallback",
    "recording.mode",
    "recording.device",
    "recording.max_seconds",
];

/// Sections whose provider may have a key in the keyring.
const KEY_SECTIONS: &[&str] = &["provider", "fallback", "cleanup"];

/// Everything the window renders.
#[derive(Debug, serde::Serialize)]
struct Snapshot {
    config: String,
    values: Map<String, Value>,
    /// Section -> whether its provider has a key stored (absent sections are left out).
    keys: Map<String, Value>,
    devices: Vec<String>,
    daemon: String,
}

fn config() -> Result<PathBuf, String> {
    settings::config_file().map_err(|e| e.to_string())
}

#[tauri::command]
async fn snapshot() -> Result<Snapshot, String> {
    let path = config()?;
    let doc = settings::read(&path).map_err(|e| e.to_string())?;
    let values = KEYS
        .iter()
        .map(|k| ((*k).to_owned(), settings::get(&doc, k)))
        .collect();
    let keys = KEY_SECTIONS
        .iter()
        .filter(|s| **s == "provider" || settings::has_section(&doc, s))
        .map(|s| {
            let stored = settings::key_stored(&path, s).unwrap_or(false);
            ((*s).to_owned(), Value::from(stored))
        })
        .collect();
    Ok(Snapshot {
        config: path.display().to_string(),
        values,
        keys,
        devices: settings::devices(),
        daemon: settings::status().await,
    })
}

/// Saves values together and returns the daemon's reload answer (`ok`, `busy`, `not_running`…).
#[tauri::command]
async fn set_values(changes: Map<String, Value>) -> Result<String, String> {
    if let Some(key) = changes.keys().find(|k| !KEYS.contains(&k.as_str())) {
        return Err(format!("{key} is not editable here"));
    }
    let changes: Vec<(&str, Value)> = changes
        .iter()
        .map(|(k, v)| (k.as_str(), v.clone()))
        .collect();
    settings::set(&config()?, &changes)?;
    Ok(settings::reload().await)
}

#[tauri::command]
async fn store_key(section: String, secret: String) -> Result<String, String> {
    settings::store_key(&config()?, &section, &secret)?;
    Ok(settings::reload().await)
}

#[tauri::command]
async fn delete_key(section: String) -> Result<String, String> {
    settings::delete_key(&config()?, &section)?;
    Ok(settings::reload().await)
}

fn main() -> ExitCode {
    let run = tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            snapshot, set_values, store_key, delete_key
        ])
        .run(tauri::generate_context!());
    if run.is_ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
