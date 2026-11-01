//! Unix socket transport.

use std::fs::OpenOptions;
use std::io::ErrorKind;
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::time::Duration;

use super::{BindError, io};

/// The worker's listening socket.
pub type Listener = UnixListener;
/// One client connection.
pub(crate) type Conn = std::os::unix::net::UnixStream;

/// Creates a missing directory as `0700`; an existing one must not be group or world accessible.
pub(super) fn prepare_dir(parent: &Path) -> Result<(), BindError> {
    if !parent.exists() {
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
    let mode = std::fs::metadata(parent)
        .map_err(|e| io(&e))?
        .permissions()
        .mode();
    if mode & 0o077 != 0 {
        return Err(BindError::Io("insecure socket directory".to_owned()));
    }
    Ok(())
}

/// Options for the `0600` lock file.
pub(super) fn lock_options() -> OpenOptions {
    let mut opts = OpenOptions::new();
    opts.create(true).truncate(false).write(true).mode(0o600);
    opts
}

/// Binds the `0600` socket, replacing a stale socket file from a dead worker.
pub(super) fn listen(path: &Path) -> Result<Listener, BindError> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_socket() => match Conn::connect(path) {
            Ok(_) => return Err(BindError::AlreadyRunning),
            Err(e) if e.kind() == ErrorKind::ConnectionRefused => {
                std::fs::remove_file(path).map_err(|e| io(&e))?;
            }
            Err(e) => return Err(io(&e)),
        },
        Ok(_) => {
            return Err(BindError::Io("path exists and is not a socket".to_owned()));
        }
        Err(e) if e.kind() == ErrorKind::NotFound => {}
        Err(e) => return Err(io(&e)),
    }
    let listener = UnixListener::bind(path).map_err(|e| io(&e))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(|e| io(&e))?;
    Ok(listener)
}

/// Makes [`accept`] poll instead of block.
pub(crate) fn prepare_listener(listener: &Listener) -> std::io::Result<()> {
    listener.set_nonblocking(true)
}

/// The next waiting client, or `None` after sleeping `wait` when there is none.
pub(crate) fn accept(listener: &Listener, wait: Duration) -> std::io::Result<Option<Conn>> {
    match listener.accept() {
        Ok((stream, _)) => Ok(Some(stream)),
        Err(e) if e.kind() == ErrorKind::WouldBlock => {
            std::thread::sleep(wait);
            Ok(None)
        }
        Err(e) => Err(e),
    }
}
