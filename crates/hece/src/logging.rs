//! Daemon logs in the state directory. Only codes, counts and timings are logged; audio,
//! transcripts, prompts and keys never reach a log call in this codebase.

use std::fs::OpenOptions;
use std::path::Path;
use std::sync::Mutex;

use tracing_subscriber::EnvFilter;

/// Above this size the log is rotated to `hece.log.1` at startup.
const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;

/// Sends `tracing` output to `<state_dir>/hece.log` (`RUST_LOG` overrides the `info` level).
/// Failure leaves logging off rather than stopping the daemon.
pub fn init(state_dir: &Path) {
    let path = state_dir.join("hece.log");
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_LOG_BYTES) {
        let _ = std::fs::rename(&path, state_dir.join("hece.log.1"));
    }
    let file = std::fs::create_dir_all(state_dir)
        .and_then(|()| OpenOptions::new().create(true).append(true).open(&path));
    let Ok(file) = file else {
        return;
    };
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .with_writer(Mutex::new(file))
        .try_init();
}
