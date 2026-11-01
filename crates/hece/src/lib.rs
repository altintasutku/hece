//! Hece CLI and daemon. Two binaries share this code: `hece` (console) and `hecew` (Windows,
//! no console window, for autostart).
#![cfg_attr(
    not(windows),
    allow(
        dead_code,
        reason = "the daemon is Windows-only; Linux support is shelved"
    )
)]

mod bench;
mod cli;
mod config;
mod dictate;
mod history;
#[cfg(windows)]
mod logging;
mod output;
mod paths;
mod providers;
#[cfg(windows)]
mod run;

use std::process::ExitCode;

use clap::Parser;
use hece_providers::{ApiKey, KeyringStore, SecretStore};

use crate::cli::{Cli, Command, HistoryArgs, KeyCommand};
use crate::config::{Config, ConfigError};

/// Some files failed (the report was still printed).
const EXIT_FILES_FAILED: u8 = 1;
/// Setup or engine error outside per-file work. Clap usage errors exit 2 on their own.
const EXIT_SETUP: u8 = 3;

/// Entry point of `hece`.
#[must_use]
pub fn console_main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(false) => ExitCode::SUCCESS,
        Ok(true) => ExitCode::from(EXIT_FILES_FAILED),
        Err(e) => {
            output::error(&format!("{e:#}"));
            ExitCode::from(EXIT_SETUP)
        }
    }
}

/// Entry point of `hecew`: the daemon without a console; problems are shown as toasts.
#[must_use]
pub fn background_main() -> ExitCode {
    #[cfg(windows)]
    {
        let result = runtime().and_then(|rt| rt.block_on(daemon(None)));
        if let Err(e) = result {
            tracing::error!(error = %e, "daemon failed");
            hece_platform::notify("Hece could not start", &format!("{e:#}"));
            return ExitCode::from(EXIT_SETUP);
        }
        ExitCode::SUCCESS
    }
    #[cfg(not(windows))]
    {
        output::error("hecew is Windows-only; use `hece run`");
        ExitCode::from(EXIT_SETUP)
    }
}

fn runtime() -> anyhow::Result<tokio::runtime::Runtime> {
    Ok(tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?)
}

fn load_config(path: Option<std::path::PathBuf>) -> anyhow::Result<Config> {
    let path = match path {
        Some(p) => p,
        None => paths::config_file()?,
    };
    match Config::load(&path) {
        Ok(cfg) => Ok(cfg),
        Err(e @ ConfigError::Created(_)) => anyhow::bail!("{e}"),
        Err(e) => Err(e.into()),
    }
}

#[cfg(windows)]
async fn daemon(config: Option<std::path::PathBuf>) -> anyhow::Result<()> {
    let state = paths::state_dir()?;
    logging::init(&state);
    let config_file = match config {
        Some(path) => path,
        None => paths::config_file()?,
    };
    let cfg = load_config(Some(config_file.clone()))?;
    let paths = run::Paths {
        config_file,
        state_dir: state,
        control: paths::control_socket()?,
    };
    run::execute(cfg, paths).await
}

fn history(args: &HistoryArgs) -> anyhow::Result<()> {
    let cfg = load_config(None)?;
    let history = history::History::new(&paths::state_dir()?, &cfg.history);
    if args.clear {
        history.clear()?;
        output::status("history deleted");
        return Ok(());
    }
    let entries = history.entries();
    for e in entries.iter().rev().take(args.count).rev() {
        let flag = if e.low_confidence {
            " (may be misheard)"
        } else {
            ""
        };
        output::line(&format!("[{}] {}{flag}", e.provider, e.text()));
    }
    Ok(())
}

fn key(command: &KeyCommand) -> anyhow::Result<()> {
    match command {
        KeyCommand::Set { provider } => {
            let key = rpassword::prompt_password(format!("API key for {provider}: "))?;
            let key = ApiKey::new(&key);
            anyhow::ensure!(!key.expose().is_empty(), "empty key, nothing stored");
            KeyringStore.set(provider, &key)?;
            output::status(&format!("stored the key for {provider} in the OS keyring"));
        }
        KeyCommand::Delete { provider } => {
            KeyringStore.delete(provider)?;
            output::status(&format!("removed the key for {provider}"));
        }
    }
    Ok(())
}

/// Returns whether any bench file failed.
fn run(cli: Cli) -> anyhow::Result<bool> {
    match cli.command {
        Command::Bench(args) => runtime()?.block_on(bench::execute(&args)),
        Command::Devices => dictate::devices().map(|()| false),
        Command::Dictate(args) => runtime()?.block_on(dictate::execute(&args)).map(|()| false),
        #[cfg(windows)]
        Command::Run(args) => runtime()?.block_on(daemon(args.config)).map(|()| false),
        #[cfg(windows)]
        Command::Status | Command::Toggle => {
            let command = if matches!(cli.command, Command::Status) {
                "status"
            } else {
                "toggle"
            };
            let socket = paths::control_socket()?;
            let answer = runtime()?.block_on(hece_platform::control_request(&socket, command))?;
            output::line(&answer);
            Ok(false)
        }
        Command::History(args) => history(&args).map(|()| false),
        #[cfg(windows)]
        Command::Repaste { raw } => {
            let cfg = load_config(None)?;
            let entries = history::History::new(&paths::state_dir()?, &cfg.history).entries();
            let last = entries
                .last()
                .ok_or_else(|| anyhow::anyhow!("history is empty"))?;
            hece_platform::copy(if raw { &last.raw } else { last.text() })?;
            output::status("copied the last dictation; paste it with Ctrl+V");
            Ok(false)
        }
        Command::Key(command) => key(&command).map(|()| false),
        #[cfg(windows)]
        Command::Autostart { action } => {
            use crate::cli::Toggle;
            match action {
                Toggle::On => {
                    let exe = std::env::current_exe()?.with_file_name("hecew.exe");
                    anyhow::ensure!(exe.is_file(), "{} not found", exe.display());
                    hece_platform::enable_autostart(&exe)?;
                    output::status("hece starts at login");
                }
                Toggle::Off => {
                    hece_platform::disable_autostart()?;
                    output::status("hece no longer starts at login");
                }
                Toggle::Status => {
                    let on = hece_platform::autostart_enabled()?;
                    output::line(if on { "on" } else { "off" });
                }
            }
            Ok(false)
        }
        Command::Config => {
            let path = paths::config_file()?;
            if let Err(ConfigError::Created(p)) = Config::load(&path) {
                output::status(&format!("created {}", p.display()));
            }
            output::line(&path.display().to_string());
            Ok(false)
        }
    }
}
