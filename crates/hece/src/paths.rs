//! Per-user locations. Windows: config under `%APPDATA%\hece`, state (history, logs, control
//! pipe name) under `%LOCALAPPDATA%\hece`; both inherit the per-user ACL of the profile.
//! Elsewhere: `$XDG_CONFIG_HOME/hece` and `$XDG_STATE_HOME/hece`.

use std::path::PathBuf;

use anyhow::Context as _;

fn dir(var: &str, xdg_fallback: &str) -> anyhow::Result<PathBuf> {
    if let Some(base) = std::env::var_os(var).filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(base).join("hece"));
    }
    let home = std::env::var_os("HOME")
        .filter(|v| !v.is_empty())
        .with_context(|| format!("neither {var} nor HOME is set"))?;
    Ok(PathBuf::from(home).join(xdg_fallback).join("hece"))
}

pub fn config_file() -> anyhow::Result<PathBuf> {
    let var = if cfg!(windows) {
        "APPDATA"
    } else {
        "XDG_CONFIG_HOME"
    };
    Ok(dir(var, ".config")?.join("config.toml"))
}

pub fn state_dir() -> anyhow::Result<PathBuf> {
    let var = if cfg!(windows) {
        "LOCALAPPDATA"
    } else {
        "XDG_STATE_HOME"
    };
    dir(var, ".local/state")
}

/// Name source of the daemon's control pipe.
#[cfg(windows)]
pub fn control_socket() -> anyhow::Result<PathBuf> {
    Ok(state_dir()?.join("daemon.sock"))
}
