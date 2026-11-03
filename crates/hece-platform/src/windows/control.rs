//! The daemon's control pipe (`hece status`, `hece toggle`): one short line in, one line out.
//! Same protection as the engine pipe: owner/SYSTEM/Administrators only, local clients only,
//! and first instance only, so a second daemon cannot share the name. Pipe names are
//! machine-wide, so clients and a failed bind check who serves the pipe
//! ([`pipe_server_is_trusted`]).

use std::path::Path;
use std::time::Duration;

use interprocess::local_socket::tokio::{Listener, Stream};
use interprocess::local_socket::traits::tokio::Listener as _;
use interprocess::local_socket::{GenericFilePath, ListenerOptions, ToFsName};
use interprocess::os::windows::local_socket::ListenerOptionsExt;
use interprocess::os::windows::security_descriptor::SecurityDescriptor;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};

use super::pipe_server_is_trusted;
use crate::PlatformError;

/// Protected DACL: full access for the pipe's owner, SYSTEM and Administrators, nobody else.
const PIPE_SDDL: &widestring::U16CStr =
    widestring::u16cstr!("D:P(A;;GA;;;OW)(A;;GA;;;SY)(A;;GA;;;BA)");
/// Longest accepted request line.
const MAX_LINE: u64 = 64;
/// A client gets this long to send its line and read the answer.
const IO_TIMEOUT: Duration = Duration::from_secs(2);

/// The daemon side of the control pipe.
#[derive(Debug)]
pub struct ControlListener(Listener);

/// One request waiting for its answer.
#[derive(Debug)]
pub struct ControlRequest {
    /// The request line, trimmed.
    pub command: String,
    stream: Stream,
}

fn pipe_name(path: &Path) -> Result<String, PlatformError> {
    super::pipe_name(path).map_err(|e| {
        tracing::warn!(kind = ?e.kind(), "pipe key unavailable");
        PlatformError::Control
    })
}

fn name(path: &Path) -> Result<interprocess::local_socket::Name<'static>, PlatformError> {
    pipe_name(path)?
        .to_fs_name::<GenericFilePath>()
        .map_err(|_| PlatformError::Control)
}

impl ControlListener {
    /// Creates the pipe for `path`; fails with [`PlatformError::AlreadyRunning`] when another
    /// daemon of this user owns it and [`PlatformError::ForeignPipe`] when another account does.
    /// Must be called inside a tokio runtime.
    pub fn bind(path: &Path) -> Result<Self, PlatformError> {
        let sd = SecurityDescriptor::deserialize(PIPE_SDDL).map_err(|_| PlatformError::Control)?;
        ListenerOptions::new()
            .name(name(path)?)
            .security_descriptor(sd)
            .create_tokio()
            .map(Self)
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::PermissionDenied
                    || e.kind() == std::io::ErrorKind::AddrInUse
                {
                    existing_owner(path)
                } else {
                    tracing::warn!(kind = ?e.kind(), "control pipe bind failed");
                    PlatformError::Control
                }
            })
    }

    /// Waits for the next request; malformed or slow clients are skipped.
    pub async fn next(&self) -> ControlRequest {
        loop {
            let Ok(stream) = self.0.accept().await else {
                continue;
            };
            let mut line = String::new();
            let read = async {
                let mut reader = BufReader::new((&stream).take(MAX_LINE));
                reader.read_line(&mut line).await
            };
            if matches!(tokio::time::timeout(IO_TIMEOUT, read).await, Ok(Ok(n)) if n > 0) {
                return ControlRequest {
                    command: line.trim().to_owned(),
                    stream,
                };
            }
        }
    }
}

impl ControlRequest {
    /// Sends the answer line.
    pub async fn reply(self, answer: &str) {
        let write = async {
            (&self.stream).write_all(answer.as_bytes()).await?;
            (&self.stream).write_all(b"\n").await
        };
        if tokio::time::timeout(IO_TIMEOUT, write).await.is_err() {
            tracing::debug!("control client did not read its answer");
        }
    }
}

/// Opens the pipe as a client. Tokio asks for `SECURITY_IDENTIFICATION`, so a server learns
/// who connected but cannot act as them.
fn connect(path: &Path) -> Result<NamedPipeClient, PlatformError> {
    let stream = ClientOptions::new().open(pipe_name(path)?).map_err(|e| {
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            PlatformError::ForeignPipe
        } else {
            PlatformError::NotRunning
        }
    })?;
    if pipe_server_is_trusted(&stream) {
        Ok(stream)
    } else {
        Err(PlatformError::ForeignPipe)
    }
}

/// Why the bind failed: our own daemon (a busy pipe is ours too) or another account's pipe.
fn existing_owner(path: &Path) -> PlatformError {
    match connect(path) {
        Err(PlatformError::ForeignPipe) => PlatformError::ForeignPipe,
        _ => PlatformError::AlreadyRunning,
    }
}

/// Sends `command` to the running daemon and returns its answer;
/// [`PlatformError::NotRunning`] when no daemon listens.
pub async fn control_request(path: &Path, command: &str) -> Result<String, PlatformError> {
    let exchange = async {
        let mut stream = connect(path)?;
        stream
            .write_all(format!("{command}\n").as_bytes())
            .await
            .map_err(|_| PlatformError::Control)?;
        let mut answer = String::new();
        BufReader::new(stream.take(1024))
            .read_line(&mut answer)
            .await
            .map_err(|_| PlatformError::Control)?;
        Ok(answer.trim().to_owned())
    };
    tokio::time::timeout(IO_TIMEOUT, exchange)
        .await
        .map_err(|_| PlatformError::Control)?
}
