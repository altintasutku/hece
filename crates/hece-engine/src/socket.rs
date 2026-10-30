//! Unix socket location and safe binding for the worker.

use std::os::unix::fs::{DirBuilderExt, FileTypeExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

use crate::error::EngineError;

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

/// `$XDG_RUNTIME_DIR/hece/engine.sock`.
pub fn socket_path() -> Result<PathBuf, EngineError> {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|d| !d.is_empty())
        .ok_or(EngineError::NoRuntimeDir)?;
    Ok(PathBuf::from(dir).join("hece").join("engine.sock"))
}

/// Binds a private (`0600`) listener at `path`, creating missing parent directories as `0700`.
/// A leftover socket file from a dead worker is replaced; a live one gives `AlreadyRunning`;
/// anything that is not a socket is never touched.
pub fn bind(path: &Path) -> Result<UnixListener, BindError> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty())
        && !parent.exists()
    {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(parent)
            .map_err(|e| io(&e))?;
        // `recursive` applies the mode to every created level, but umask can still narrow
        // it; the leaf must be exactly 0700.
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| io(&e))?;
    }
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_socket() => match UnixStream::connect(path) {
            Ok(_) => return Err(BindError::AlreadyRunning),
            Err(e) if e.kind() == std::io::ErrorKind::ConnectionRefused => {
                std::fs::remove_file(path).map_err(|e| io(&e))?;
            }
            Err(e) => return Err(io(&e)),
        },
        Ok(_) => {
            return Err(BindError::Io("path exists and is not a socket".to_owned()));
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(io(&e)),
    }
    let listener = UnixListener::bind(path).map_err(|e| io(&e))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(|e| io(&e))?;
    Ok(listener)
}
