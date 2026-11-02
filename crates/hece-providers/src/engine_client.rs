//! Daemon-side client for the engine worker: spawn on demand, handshake, prewarm, serialized
//! requests with timeouts, and typed failures. Requests and responses are never logged beyond
//! codes and sizes (they carry audio, transcripts and personal vocabulary).

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use hece_core::ipc::{
    AudioB64, CONN_IDLE_TIMEOUT, DEFAULT_COMPUTE, LoadRequest, MAX_AUDIO_SAMPLES, PROTOCOL_VERSION,
    Request, Response, TranscribeRequest, decode_body, encode_frame, frame_len,
};
use hece_core::{Transcript, Vocabulary};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
#[cfg(unix)]
use tokio::net::UnixStream as Stream;
#[cfg(windows)]
use tokio::net::windows::named_pipe::NamedPipeClient as Stream;
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tokio::time::{Instant, sleep, timeout};

/// Connections idle for longer than this are replaced before sending, a margin below the
/// worker's own idle timeout.
const RECONNECT_MARGIN: Duration = Duration::from_secs(5);

const RETRY_INTERVAL: Duration = Duration::from_millis(100);
/// Pause before retrying a named pipe whose instances are all busy.
#[cfg(windows)]
const PIPE_BUSY_RETRY: Duration = Duration::from_millis(10);
/// How long to wait for a worker that won the bind race before spawning another.
const RESPAWN_GRACE: Duration = Duration::from_secs(1);
/// Worker exit code meaning another worker already owns the socket.
const EXIT_ALREADY_RUNNING: i32 = 3;
/// `CREATE_NEW_PROCESS_GROUP` process creation flag.
#[cfg(windows)]
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
/// `CREATE_NO_WINDOW` process creation flag.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// Windows `ERROR_PIPE_BUSY`: the worker is alive but every pipe instance is taken.
#[cfg(windows)]
const ERROR_PIPE_BUSY: i32 = 231;
/// Environment variable naming the per-user runtime directory.
#[cfg(unix)]
const RUNTIME_DIR_VAR: &str = "XDG_RUNTIME_DIR";
/// Environment variable naming the per-user runtime directory.
#[cfg(windows)]
const RUNTIME_DIR_VAR: &str = "LOCALAPPDATA";
/// Samples per second of the wire audio format.
const SAMPLE_RATE: u64 = 16_000;

/// Failures of the engine client. [`EngineClientError::code`] is the stable user-facing code.
#[derive(Debug, thiserror::Error)]
pub enum EngineClientError {
    /// The engine binary could not be found.
    #[error("engine binary not found")]
    EngineMissing,
    /// Starting the engine process failed.
    #[error("failed to start the engine: {0}")]
    Spawn(String),
    /// The worker socket did not come up in time.
    #[error("timed out connecting to the engine")]
    ConnectTimeout,
    /// The worker speaks a different protocol version.
    #[error("engine protocol version {worker} is not supported")]
    ProtocolMismatch {
        /// Protocol version announced by the worker.
        worker: u32,
    },
    /// The worker closed the connection mid-request.
    #[error("the engine stopped unexpectedly")]
    Crashed,
    /// The worker did not answer in time.
    #[error("the engine did not answer in time")]
    Timeout,
    /// The engine reported a failure; only its stable code is known.
    #[error("engine error: {code}")]
    Engine {
        /// Stable error code from the engine.
        code: String,
    },
    /// The audio is longer than the protocol allows.
    #[error("audio is too long for the engine")]
    InvalidAudio,
    /// The worker sent something unexpected.
    #[error("unexpected engine response: {0}")]
    Protocol(String),
    /// Socket I/O failed.
    #[error("engine i/o failed: {0}")]
    Io(String),
    /// `XDG_RUNTIME_DIR` (`LOCALAPPDATA` on Windows) is not set, so there is no safe socket
    /// location.
    #[error("no runtime directory available for the engine socket")]
    NoRuntimeDir,
    /// The engine pipe belongs to another account, so nothing is sent to it.
    #[error("another user holds the engine pipe")]
    ForeignPipe,
}

impl EngineClientError {
    /// Stable error code safe to show to users.
    #[must_use]
    pub fn code(&self) -> &str {
        match self {
            Self::EngineMissing => "engine_missing",
            Self::Spawn(_) => "engine_spawn_failed",
            Self::ConnectTimeout => "engine_connect_timeout",
            Self::ProtocolMismatch { .. } => "engine_protocol_mismatch",
            Self::Crashed => "engine_crashed",
            Self::Timeout => "engine_timeout",
            Self::Engine { code } => code,
            Self::InvalidAudio => "invalid_audio",
            Self::Protocol(_) => "engine_protocol",
            Self::Io(_) => "engine_io",
            Self::NoRuntimeDir => "no_runtime_dir",
            Self::ForeignPipe => "foreign_pipe",
        }
    }
}

/// How to reach and start the engine worker.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// Explicit path of the `hece-engine` binary; otherwise next to the current exe.
    pub engine_path: Option<PathBuf>,
    /// Socket path of the worker; on Windows the worker listens on
    /// [`hece_core::ipc::pipe_name`] of this path.
    pub socket: PathBuf,
    /// Directory holding the `CTranslate2` model.
    pub model_dir: PathBuf,
    /// Compute type, e.g. `int8_float16`.
    pub compute: String,
    /// Idle time after which the worker exits.
    pub idle_ttl: Duration,
    /// How long to wait for the socket after spawning (default 20 s).
    pub connect_timeout: Duration,
    /// Request timeout before the per-audio-length allowance (default 120 s).
    pub base_timeout: Duration,
    /// A connection idle for longer than this is replaced before the next request, so the
    /// request never lands on one the worker is about to close (default 25 s).
    pub reconnect_after: Duration,
    /// Beam width sent with every request; `None` keeps the engine default.
    pub beam_size: Option<u32>,
}

impl EngineConfig {
    /// Defaults: socket `$XDG_RUNTIME_DIR/hece/engine.sock` (`%LOCALAPPDATA%\hece\engine.sock` on
    /// Windows), idle TTL 30 min, connect 20 s, base request timeout 120 s.
    pub fn default_for(model_dir: PathBuf) -> Result<Self, EngineClientError> {
        Self::with_runtime_dir(model_dir, std::env::var_os(RUNTIME_DIR_VAR))
    }

    /// Like [`EngineConfig::default_for`] with the runtime directory given explicitly; unset or
    /// empty is [`EngineClientError::NoRuntimeDir`] (there is no `/tmp` fallback).
    pub fn with_runtime_dir(
        model_dir: PathBuf,
        runtime_dir: Option<std::ffi::OsString>,
    ) -> Result<Self, EngineClientError> {
        let runtime = runtime_dir
            .filter(|v| !v.is_empty())
            .ok_or(EngineClientError::NoRuntimeDir)?;
        Ok(Self {
            engine_path: None,
            socket: PathBuf::from(runtime).join("hece").join("engine.sock"),
            model_dir,
            compute: DEFAULT_COMPUTE.to_owned(),
            idle_ttl: Duration::from_mins(30),
            connect_timeout: Duration::from_secs(20),
            base_timeout: Duration::from_secs(120),
            reconnect_after: CONN_IDLE_TIMEOUT.saturating_sub(RECONNECT_MARGIN),
            beam_size: None,
        })
    }
}

/// Starts the worker process.
pub trait Spawner: Send + Sync {
    /// Spawns a worker for `cfg`; it must listen on `cfg.socket`.
    fn spawn(&self, cfg: &EngineConfig) -> Result<Child, EngineClientError>;
}

/// Spawns the real `hece-engine serve` process.
#[derive(Debug, Clone, Copy, Default)]
pub struct ProcessSpawner;

impl ProcessSpawner {
    fn resolve(cfg: &EngineConfig) -> Result<PathBuf, EngineClientError> {
        if let Some(path) = &cfg.engine_path {
            return if path.is_file() {
                Ok(path.clone())
            } else {
                Err(EngineClientError::EngineMissing)
            };
        }
        // Only next to this binary: a PATH lookup could pick up any hece-engine.
        std::env::current_exe()
            .ok()
            .and_then(|exe| {
                exe.parent()
                    .map(|d| d.join(format!("hece-engine{}", std::env::consts::EXE_SUFFIX)))
            })
            .filter(|p| p.is_file())
            .ok_or(EngineClientError::EngineMissing)
    }
}

impl Spawner for ProcessSpawner {
    fn spawn(&self, cfg: &EngineConfig) -> Result<Child, EngineClientError> {
        let program = Self::resolve(cfg)?;
        let mut command = Command::new(program);
        command
            .arg("serve")
            .arg("--socket")
            .arg(&cfg.socket)
            .arg("--idle-ttl-secs")
            .arg(cfg.idle_ttl.as_secs().to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit());
        // Own process group: a signal aimed at the daemon's group does not reach the worker.
        #[cfg(unix)]
        command.process_group(0);
        // Windows: Ctrl+C aimed at the daemon's console does not reach the worker, and the
        // worker never opens a console window of its own.
        #[cfg(windows)]
        command.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
        command
            // The worker keeps its loaded model across a daemon restart.
            .kill_on_drop(false)
            .spawn()
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    EngineClientError::EngineMissing
                } else {
                    EngineClientError::Spawn(format!("{:?}", e.kind()))
                }
            })
    }
}

#[derive(Debug, Default)]
struct State {
    conn: Option<Stream>,
    /// The worker this client started, if any; only that one is ever killed.
    child: Option<Child>,
    /// When `conn` last completed a request.
    last_used: Option<Instant>,
}

/// Client for the engine worker. Requests are serialized: concurrent callers queue in order.
pub struct EngineClient {
    cfg: EngineConfig,
    spawner: Arc<dyn Spawner>,
    state: Mutex<State>,
}

impl std::fmt::Debug for EngineClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineClient")
            .field("socket", &self.cfg.socket)
            .finish_non_exhaustive()
    }
}

/// Where a round trip failed; only a failed request write may be resent.
enum Fail {
    /// Writing the request failed, so the worker never saw all of it.
    Write(EngineClientError),
    /// Anything after the request was fully written.
    Later(EngineClientError),
}

impl Fail {
    fn into_error(self) -> EngineClientError {
        match self {
            Self::Write(e) | Self::Later(e) => e,
        }
    }
}

/// Connects to the worker socket; `None` when no worker listens.
#[cfg(unix)]
async fn open(socket: &Path, _deadline: Instant) -> Option<Stream> {
    Stream::connect(socket).await.ok()
}

/// Connects to the worker's named pipe; `None` when no worker listens. While every instance
/// is busy the worker is alive, so this waits (until `deadline`) instead of reporting `None`,
/// which would spawn another worker.
#[cfg(windows)]
async fn open(socket: &Path, deadline: Instant) -> Option<Stream> {
    let name = hece_core::ipc::pipe_name(socket);
    loop {
        match tokio::net::windows::named_pipe::ClientOptions::new().open(&name) {
            Ok(stream) => return Some(stream),
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) && Instant::now() < deadline => {
                sleep(PIPE_BUSY_RETRY).await;
            }
            Err(_) => return None,
        }
    }
}

fn io_error(e: &std::io::Error) -> EngineClientError {
    use std::io::ErrorKind::{BrokenPipe, ConnectionAborted, ConnectionReset, UnexpectedEof};
    match e.kind() {
        UnexpectedEof | ConnectionReset | ConnectionAborted | BrokenPipe => {
            EngineClientError::Crashed
        }
        kind => EngineClientError::Io(format!("{kind:?}")),
    }
}

fn protocol_error(e: &hece_core::ipc::IpcError) -> EngineClientError {
    EngineClientError::Protocol(e.code().to_owned())
}

async fn round_trip(conn: &mut Stream, req: &Request) -> Result<Response, Fail> {
    let frame = encode_frame(req).map_err(|e| Fail::Write(protocol_error(&e)))?;
    conn.write_all(&frame)
        .await
        .map_err(|e| Fail::Write(io_error(&e)))?;
    let mut header = [0u8; 4];
    conn.read_exact(&mut header)
        .await
        .map_err(|e| Fail::Later(io_error(&e)))?;
    let len = frame_len(header).map_err(|e| Fail::Later(protocol_error(&e)))?;
    let mut body = vec![0u8; len];
    conn.read_exact(&mut body)
        .await
        .map_err(|e| Fail::Later(io_error(&e)))?;
    decode_body(&body).map_err(|e| Fail::Later(protocol_error(&e)))
}

/// Error codes the worker sends right before closing the connection.
fn closes_connection(code: &str) -> bool {
    matches!(
        code,
        "bad_json" | "frame_too_large" | "bad_audio" | "invalid_audio"
    )
}

impl EngineClient {
    /// Creates a client; nothing is spawned or connected until the first call.
    #[must_use]
    pub fn new(cfg: EngineConfig, spawner: Arc<dyn Spawner>) -> Self {
        Self {
            cfg,
            spawner,
            state: Mutex::new(State::default()),
        }
    }

    /// Connects (spawning the worker if needed) and loads the model; returns after `Loaded`.
    pub async fn prewarm(&self) -> Result<(), EngineClientError> {
        let req = Request::Load(LoadRequest {
            model_dir: self.cfg.model_dir.clone(),
            compute: self.cfg.compute.clone(),
        });
        match self.call(&req, self.cfg.base_timeout, true).await? {
            Some(Response::Loaded { load_ms }) => {
                tracing::debug!(load_ms, "engine model loaded");
                Ok(())
            }
            Some(other) => Err(unexpected(&other)),
            None => Err(EngineClientError::Crashed),
        }
    }

    /// Transcribes 16 kHz mono samples. The caller keeps ownership of `audio`.
    pub async fn transcribe(
        &self,
        audio: &[i16],
        language: Option<&str>,
        vocabulary: Option<&Vocabulary>,
        prompt_prefix: &str,
        prompt_tokens: usize,
    ) -> Result<Transcript, EngineClientError> {
        if audio.len() > MAX_AUDIO_SAMPLES {
            return Err(EngineClientError::InvalidAudio);
        }
        let req = Request::Transcribe(TranscribeRequest {
            model_dir: self.cfg.model_dir.clone(),
            compute: self.cfg.compute.clone(),
            language: language.map(str::to_owned),
            vocabulary: vocabulary.cloned(),
            prompt_prefix: prompt_prefix.to_owned(),
            prompt_tokens,
            vad: true,
            beam_size: self.cfg.beam_size,
            audio: AudioB64::encode(audio),
        });
        // One extra second per ten seconds of audio.
        let samples = u64::try_from(audio.len()).unwrap_or(u64::MAX);
        let extra = Duration::from_millis(samples.saturating_mul(1000) / SAMPLE_RATE / 10);
        let limit = self.cfg.base_timeout.saturating_add(extra);
        tracing::debug!(samples, "sending transcribe request");
        match self.call(&req, limit, true).await? {
            Some(Response::Transcript(t)) => Ok(t),
            Some(other) => Err(unexpected(&other)),
            None => Err(EngineClientError::Crashed),
        }
    }

    /// Asks a running worker to exit. Succeeds when no worker is reachable or it already died.
    pub async fn shutdown(&self) -> Result<(), EngineClientError> {
        let mut state = self.state.lock().await;
        let result = self
            .call_locked(&mut state, &Request::Shutdown, self.cfg.base_timeout, false)
            .await;
        state.conn = None;
        state.child = None;
        match result {
            Ok(Some(Response::Bye) | None) | Err(EngineClientError::Crashed) => Ok(()),
            Ok(Some(other)) => Err(unexpected(&other)),
            Err(e) => Err(e),
        }
    }

    async fn call(
        &self,
        req: &Request,
        limit: Duration,
        allow_spawn: bool,
    ) -> Result<Option<Response>, EngineClientError> {
        let mut state = self.state.lock().await;
        self.call_locked(&mut state, req, limit, allow_spawn).await
    }

    /// Sends one request on the (possibly new) connection. `Ok(None)` means no worker was
    /// reachable and spawning was not allowed. The connection is kept only after a clean answer.
    /// A reused connection that fails while the request is being written (e.g. the worker
    /// exited on its idle TTL) is replaced once and the request is resent; a failure while
    /// waiting for the response is never resent.
    async fn call_locked(
        &self,
        state: &mut State,
        req: &Request,
        limit: Duration,
        allow_spawn: bool,
    ) -> Result<Option<Response>, EngineClientError> {
        Self::reap_child(state);
        if let Some(used) = state.last_used
            && state.conn.is_some()
            && used.elapsed() > self.cfg.reconnect_after
        {
            tracing::debug!("engine connection idle, reconnecting");
            state.conn = None;
        }
        let mut retried = false;
        loop {
            let (mut conn, reused) = if let Some(conn) = state.conn.take() {
                (conn, true)
            } else {
                match self.connect(state, allow_spawn).await? {
                    Some(conn) => (conn, false),
                    None => return Ok(None),
                }
            };
            match timeout(limit, round_trip(&mut conn, req)).await {
                Ok(Ok(Response::Error { code })) => {
                    // The worker closes after framing errors; only backend errors keep it.
                    if !closes_connection(&code) {
                        state.conn = Some(conn);
                        state.last_used = Some(Instant::now());
                    }
                    return Err(EngineClientError::Engine { code });
                }
                Ok(Ok(resp)) => {
                    state.conn = Some(conn);
                    state.last_used = Some(Instant::now());
                    return Ok(Some(resp));
                }
                Ok(Err(Fail::Write(EngineClientError::Crashed))) if reused && !retried => {
                    tracing::debug!("stale engine connection, reconnecting");
                    retried = true;
                }
                Ok(Err(fail)) => {
                    let e = fail.into_error();
                    tracing::warn!(code = e.code(), "engine request failed");
                    return Err(e);
                }
                Err(_) => {
                    tracing::warn!("engine request timed out");
                    Self::kill_child(state).await;
                    return Err(EngineClientError::Timeout);
                }
            }
        }
    }

    /// Reaps a worker this client started that has since exited (e.g. on its idle TTL), so it
    /// does not linger as a zombie, and drops the connection that belonged to it.
    fn reap_child(state: &mut State) {
        let Some(child) = state.child.as_mut() else {
            return;
        };
        match child.try_wait() {
            Ok(Some(_)) => {
                state.child = None;
                state.conn = None;
            }
            Ok(None) => {}
            Err(e) => tracing::debug!(kind = ?e.kind(), "try_wait on engine child failed"),
        }
    }

    /// Kills and reaps the worker this client started, if any.
    async fn kill_child(state: &mut State) {
        if let Some(mut child) = state.child.take()
            && let Err(e) = child.kill().await
        {
            tracing::warn!(kind = ?e.kind(), "failed to kill engine");
        }
    }

    /// Connects to the socket, spawning the worker if allowed, then performs the handshake.
    async fn connect(
        &self,
        state: &mut State,
        allow_spawn: bool,
    ) -> Result<Option<Stream>, EngineClientError> {
        let deadline = Instant::now() + self.cfg.connect_timeout;
        let mut spawned = false;
        // After losing the bind race to another worker, give that one time to come up before
        // spawning again (it may also have exited on its idle TTL in the meantime).
        let mut respawn_after = Instant::now();
        let mut stream = loop {
            if let Some(stream) = open(&self.cfg.socket, deadline).await {
                break stream;
            }
            if !allow_spawn {
                return Ok(None);
            }
            if spawned {
                if Self::check_child(state)? {
                    // The worker we started lost the race to another one that is still
                    // coming up. If that one exits too, a later pass may spawn again.
                    spawned = false;
                    respawn_after = Instant::now() + RESPAWN_GRACE;
                }
            } else if Instant::now() >= respawn_after {
                state.child = Some(self.spawner.spawn(&self.cfg)?);
                spawned = true;
                tracing::debug!("spawned engine worker");
                continue;
            }
            if Instant::now() >= deadline {
                // A worker we started that never bound the socket must not pile up.
                Self::kill_child(state).await;
                return Err(EngineClientError::ConnectTimeout);
            }
            sleep(RETRY_INTERVAL).await;
        };
        // The pipe name is predictable and machine-wide: never send audio to another account.
        #[cfg(windows)]
        if !hece_platform::pipe_server_is_current_user(&stream) {
            tracing::warn!("engine pipe is held by another user");
            Self::kill_child(state).await;
            return Err(EngineClientError::ForeignPipe);
        }
        // The socket answered, so a worker is alive but may be busy (for example loading a
        // model for another client); give it the connect budget plus a full request budget.
        let hello_budget = self
            .cfg
            .connect_timeout
            .saturating_add(self.cfg.base_timeout);
        let Ok(hello) = timeout(hello_budget, round_trip(&mut stream, &Request::Hello)).await
        else {
            tracing::warn!("engine did not answer the handshake");
            Self::kill_child(state).await;
            return Err(EngineClientError::Timeout);
        };
        let hello = hello.map_err(Fail::into_error)?;
        match hello {
            Response::Hello { protocol, .. } if protocol == PROTOCOL_VERSION => Ok(Some(stream)),
            Response::Hello { protocol, .. } => {
                Err(EngineClientError::ProtocolMismatch { worker: protocol })
            }
            other => Err(unexpected(&other)),
        }
    }

    /// Notices a freshly spawned worker that died before binding the socket. `Ok(true)` means
    /// it exited with [`EXIT_ALREADY_RUNNING`] because another worker owns the socket.
    fn check_child(state: &mut State) -> Result<bool, EngineClientError> {
        let Some(child) = state.child.as_mut() else {
            return Ok(false);
        };
        match child.try_wait() {
            Ok(Some(status)) if status.code() == Some(EXIT_ALREADY_RUNNING) => {
                // Another worker owns the socket; keep retrying the connect.
                state.child = None;
                Ok(true)
            }
            Ok(Some(status)) => {
                state.child = None;
                Err(EngineClientError::Spawn(format!(
                    "worker exited with code {:?}",
                    status.code()
                )))
            }
            Ok(None) => Ok(false),
            Err(e) => {
                tracing::debug!(kind = ?e.kind(), "try_wait on engine child failed");
                Ok(false)
            }
        }
    }
}

fn unexpected(resp: &Response) -> EngineClientError {
    let kind = match resp {
        Response::Hello { .. } => "hello",
        Response::Loaded { .. } => "loaded",
        Response::Transcript(_) => "transcript",
        Response::Error { .. } => "error",
        Response::Bye => "bye",
    };
    EngineClientError::Protocol(format!("unexpected {kind} response"))
}
