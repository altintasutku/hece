//! Hece settings window. Edits `config.toml` through `hece::settings` (comments kept, invalid
//! edits refused), keeps keys in the Credential Manager and asks the daemon to reload after
//! every change. Keys never travel back to the window; it only learns whether one is stored.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::process::ExitCode;

use std::ops::ControlFlow;
use std::sync::atomic::{AtomicBool, Ordering};

use hece::{models, settings};
use serde_json::{Map, Value};
use tauri::ipc::Channel;

/// Every config value the window shows or edits.
const KEYS: &[&str] = &[
    "provider.kind",
    "provider.name",
    "provider.model_dir",
    "provider.base_url",
    "provider.model",
    "provider.key",
    "provider.segments",
    "transcription.language",
    "transcription.allow_cloud_fallback",
    "recording.mode",
    "recording.device",
    "recording.max_seconds",
    "ui.overlay",
    "updates.check",
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
    models: Vec<ModelInfo>,
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
        models: models(),
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

/// A catalog model as the window shows it.
#[derive(Debug, serde::Serialize)]
struct ModelInfo {
    id: &'static str,
    name: &'static str,
    /// Download size in MB.
    size_mb: u64,
    /// Folder it is (or will be) downloaded to.
    path: String,
    installed: bool,
}

fn models() -> Vec<ModelInfo> {
    let Ok(dir) = models::models_dir() else {
        return Vec::new();
    };
    models::CATALOG
        .iter()
        .map(|m| ModelInfo {
            id: m.id,
            name: m.name,
            size_mb: m.size() / 1_000_000,
            path: dir.join(m.id).display().to_string(),
            installed: models::installed(m, &dir),
        })
        .collect()
}

/// Set while a download runs; clearing it cancels the download.
static DOWNLOADING: AtomicBool = AtomicBool::new(false);

/// Downloads a catalog model (resuming an earlier attempt), reporting the percentage on
/// `progress`, then makes it the local model. Returns the daemon's reload answer, or an error
/// code.
#[tauri::command]
async fn download_model(id: String, progress: Channel<u64>) -> Result<String, String> {
    let model = models::find(&id).ok_or("unknown_model")?;
    let dir = models::models_dir().map_err(|_| "model_disk_error")?;
    if DOWNLOADING.swap(true, Ordering::SeqCst) {
        return Err("model_download_running".to_owned());
    }
    let mut last = u64::MAX;
    let result = models::download(&models::client(), models::HUB, model, &dir, |p| {
        let percent = p.done * 100 / p.total.max(1);
        if percent != last {
            last = percent;
            let _ = progress.send(percent);
        }
        if DOWNLOADING.load(Ordering::SeqCst) {
            ControlFlow::Continue(())
        } else {
            ControlFlow::Break(())
        }
    })
    .await;
    DOWNLOADING.store(false, Ordering::SeqCst);
    let path = result.map_err(|e| e.code().to_owned())?;
    let changes = [(
        "provider.model_dir",
        Value::from(path.display().to_string()),
    )];
    settings::set(&config()?, &changes)?;
    Ok(settings::reload().await)
}

/// Cancels a running download; what arrived is kept for the next try.
#[tauri::command]
fn cancel_download() {
    DOWNLOADING.store(false, Ordering::SeqCst);
}

/// Starts the daemon (`hecew.exe`, installed next to this window).
#[tauri::command]
fn start_daemon() -> Result<(), String> {
    let hecew = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|d| d.join("hecew.exe")))
        .filter(|p| p.is_file())
        .ok_or("hecew.exe is not next to the settings window")?;
    std::process::Command::new(hecew)
        .spawn()
        .map(drop)
        .map_err(|e| format!("could not start Hece ({:?})", e.kind()))
}

fn main() -> ExitCode {
    let run = tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            snapshot,
            set_values,
            store_key,
            delete_key,
            start_daemon,
            download_model,
            cancel_download
        ])
        .run(tauri::generate_context!());
    if run.is_ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
