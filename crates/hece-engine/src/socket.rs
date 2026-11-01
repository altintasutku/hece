//! Worker socket location and safe binding: a unix socket, or on Windows a named pipe derived
//! from the same path (see [`hece_core::ipc::pipe_name`]).

use std::ffi::OsString;
use std::fs::File;
use std::path::{Path, PathBuf};

use crate::error::EngineError;

#[cfg(windows)]
mod pipe;
#[cfg(unix)]
mod unix;

#[cfg(windows)]
use pipe as platform;
#[cfg(unix)]
use unix as platform;

pub use platform::Listener;
pub(crate) use platform::{Conn, accept, prepare_listener};

/// Environment variable naming the per-user runtime directory.
#[cfg(unix)]
const RUNTIME_DIR_VAR: &str = "XDG_RUNTIME_DIR";
/// Environment variable naming the per-user runtime directory.
#[cfg(windows)]
const RUNTIME_DIR_VAR: &str = "LOCALAPPDATA";

/// Failures from [`bind`].
#[derive(Debug, thiserror::Error)]
pub enum BindError {
    /// Another worker is already listening on the path.
    #[error("another worker is already running")]
    AlreadyRunning,
    /// Creating the directory, replacing a stale file or binding failed.
    #[error("socket setup failed: {0}")]
    Io(String),
}

fn io(e: &std::io::Error) -> BindError {
    BindError::Io(e.to_string())
}

/// `$XDG_RUNTIME_DIR/hece/engine.sock`. On Windows `%LOCALAPPDATA%\hece\engine.sock`: only the
/// lock file lives in that directory, the worker listens on the matching named pipe.
pub fn socket_path() -> Result<PathBuf, EngineError> {
    socket_path_from(std::env::var_os(RUNTIME_DIR_VAR))
}

/// [`socket_path`] for an explicit runtime directory value; unset or empty is `NoRuntimeDir`.
pub fn socket_path_from(runtime_dir: Option<OsString>) -> Result<PathBuf, EngineError> {
    let dir = runtime_dir
        .filter(|d| !d.is_empty())
        .ok_or(EngineError::NoRuntimeDir)?;
    Ok(PathBuf::from(dir).join("hece").join("engine.sock"))
}

/// A bound listener plus the lock that keeps other workers out. Keep it alive while serving;
/// dropping it releases the lock.
#[derive(Debug)]
pub struct Bound {
    /// The listening socket.
    pub listener: Listener,
    /// Exclusive `flock` on the sibling `engine.lock` file.
    pub lock: File,
}

/// Binds a private listener for `path`. An exclusive lock on `engine.lock` next to the socket
/// is taken first so two workers cannot race to bind.
///
/// On unix the socket is `0600` and missing parent directories are created as `0700`; an
/// existing parent directory must not be group or world accessible. A leftover socket file from
/// a dead worker is replaced; a live one gives `AlreadyRunning`; anything that is not a socket is
/// never touched.
///
/// On Windows the directory inherits the per-user ACL of `%LOCALAPPDATA%`. The named pipe
/// admits only its owner, SYSTEM and Administrators, rejects remote clients and must be the
/// first instance of its name, so a process squatting on the name makes binding fail.
pub fn bind(path: &Path) -> Result<Bound, BindError> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        platform::prepare_dir(parent)?;
    }
    let lock = platform::lock_options()
        .open(path.with_file_name("engine.lock"))
        .map_err(|e| io(&e))?;
    match lock.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => return Err(BindError::AlreadyRunning),
        Err(std::fs::TryLockError::Error(e)) => return Err(io(&e)),
    }
    let listener = platform::listen(path)?;
    Ok(Bound { listener, lock })
}
