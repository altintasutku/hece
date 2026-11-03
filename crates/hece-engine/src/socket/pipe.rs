//! Windows named pipe transport. Named pipes have no blocking I/O timeouts, so each read and
//! write runs on a private single-threaded tokio runtime under `tokio::time::timeout`; the
//! server code above sees the same blocking API as a unix stream.

use std::cell::Cell;
use std::fs::OpenOptions;
use std::io::{self, ErrorKind, Read, Write};
use std::net::Shutdown;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use interprocess::local_socket::tokio::{Listener as PipeListener, Stream as PipeStream};
use interprocess::local_socket::traits::tokio::Listener as _;
use interprocess::local_socket::{GenericFilePath, ListenerOptions, ToFsName};
use interprocess::os::windows::local_socket::ListenerOptionsExt;
use interprocess::os::windows::security_descriptor::SecurityDescriptor;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::runtime::Runtime;

use super::{BindError, io};

/// Protected DACL: full access for the pipe's owner, SYSTEM and Administrators, nobody else.
const PIPE_SDDL: &widestring::U16CStr =
    widestring::u16cstr!("D:P(A;;GA;;;OW)(A;;GA;;;SY)(A;;GA;;;BA)");

/// The worker's listening named pipe.
#[derive(Debug)]
pub struct Listener {
    rt: Arc<Runtime>,
    inner: PipeListener,
}

/// One client connection.
#[derive(Debug)]
pub(crate) struct Conn {
    rt: Arc<Runtime>,
    stream: PipeStream,
    read_timeout: Cell<Option<Duration>>,
    write_timeout: Cell<Option<Duration>>,
}

/// Creates a missing directory; it inherits the per-user ACL of `%LOCALAPPDATA%`.
pub(super) fn prepare_dir(parent: &Path) -> Result<(), BindError> {
    std::fs::create_dir_all(parent).map_err(|e| io(&e))
}

/// Options for the lock file.
pub(super) fn lock_options() -> OpenOptions {
    let mut opts = OpenOptions::new();
    opts.create(true).truncate(false).write(true);
    opts
}

/// Creates the first instance of the pipe for `path`. An existing pipe of that name (a worker
/// that has not released it yet, or a squatter) makes this fail instead of being shared.
pub(super) fn listen(path: &Path) -> Result<Listener, BindError> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
        .map_err(|e| io(&e))?;
    let sd = SecurityDescriptor::deserialize(PIPE_SDDL).map_err(|e| io(&e))?;
    let name = hece_platform::pipe_name(path)
        .map_err(|e| io(&e))?
        .to_fs_name::<GenericFilePath>()
        .map_err(|e| io(&e))?;
    let options = ListenerOptions::new().name(name).security_descriptor(sd);
    let inner = {
        let _guard = rt.enter();
        options.create_tokio().map_err(|e| io(&e))?
    };
    Ok(Listener {
        rt: Arc::new(rt),
        inner,
    })
}

/// Nothing to do: [`accept`] already waits with a timeout.
#[expect(
    clippy::unnecessary_wraps,
    reason = "same signature as the unix transport"
)]
pub(crate) fn prepare_listener(_listener: &Listener) -> io::Result<()> {
    Ok(())
}

/// Waits up to `wait` for the next client; `None` when none arrived.
pub(crate) fn accept(listener: &Listener, wait: Duration) -> io::Result<Option<Conn>> {
    // `accept` is cancel safe: a client that connects as the timeout fires is picked up by the
    // next call.
    let accepted = listener
        .rt
        .block_on(async { tokio::time::timeout(wait, listener.inner.accept()).await });
    match accepted {
        Ok(Ok(stream)) => Ok(Some(Conn {
            rt: Arc::clone(&listener.rt),
            stream,
            read_timeout: Cell::new(None),
            write_timeout: Cell::new(None),
        })),
        Ok(Err(e)) => Err(e),
        Err(_) => Ok(None),
    }
}

/// Runs `op` to completion on `rt`, failing with `TimedOut` after `limit`.
fn timed<T>(
    rt: &Runtime,
    limit: Option<Duration>,
    op: impl Future<Output = io::Result<T>>,
) -> io::Result<T> {
    rt.block_on(async {
        match limit {
            Some(limit) => tokio::time::timeout(limit, op)
                .await
                .unwrap_or_else(|_| Err(ErrorKind::TimedOut.into())),
            None => op.await,
        }
    })
}

#[expect(
    clippy::unnecessary_wraps,
    clippy::unused_self,
    reason = "mirrors the std::os::unix::net::UnixStream methods the server calls"
)]
impl Conn {
    /// Always blocking from the caller's view; kept for parity with the unix stream.
    pub(crate) fn set_nonblocking(&self, _nonblocking: bool) -> io::Result<()> {
        Ok(())
    }

    /// Longest wait for a read; `None` waits forever.
    pub(crate) fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        self.read_timeout.set(timeout);
        Ok(())
    }

    /// Longest wait for a write; `None` waits forever.
    pub(crate) fn set_write_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        self.write_timeout.set(timeout);
        Ok(())
    }

    /// Named pipes cannot be half closed; a no-op.
    pub(crate) fn shutdown(&self, _how: Shutdown) -> io::Result<()> {
        Ok(())
    }
}

impl Read for Conn {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        timed(&self.rt, self.read_timeout.get(), self.stream.read(buf))
    }
}

impl Write for Conn {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        timed(&self.rt, self.write_timeout.get(), self.stream.write(buf))
    }

    fn flush(&mut self) -> io::Result<()> {
        timed(&self.rt, self.write_timeout.get(), self.stream.flush())
    }
}
