//! Daemon-side client for the engine worker: spawn on demand, handshake, prewarm, serialized
//! requests with timeouts, and typed failures. Requests and responses are never logged beyond
//! codes and sizes (they carry audio, transcripts and personal vocabulary).

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use hece_core::ipc::{
    AudioB64, DEFAULT_COMPUTE, LoadRequest, PROTOCOL_VERSION, Request, Response, TranscribeRequest,
    decode_body, encode_frame, frame_len,
};
use hece_core::{Transcript, Vocabulary};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tokio::time::{Instant, sleep, timeout};

const RETRY_INTERVAL: Duration = Duration::from_millis(100);
/// Worker exit code meaning another worker already owns the socket.
const EXIT_ALREADY_RUNNING: i32 = 3;
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
    /// The worker sent something unexpected.
    #[error("unexpected engine response: {0}")]
    Protocol(String),
    /// Socket I/O failed.
    #[error("engine i/o failed: {0}")]
    Io(String),
    /// `XDG_RUNTIME_DIR` is not set, so there is no safe socket location.
    #[error("no runtime directory available for the engine socket")]
    NoRuntimeDir,
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
            Self::Protocol(_) => "engine_protocol",
            Self::Io(_) => "engine_io",
            Self::NoRuntimeDir => "no_runtime_dir",
        }
    }
}

/// How to reach and start the engine worker.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// Explicit path of the `hece-engine` binary; otherwise next to the current exe, then `PATH`.
    pub engine_path: Option<PathBuf>,
    /// Unix socket path of the worker.
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
}

impl EngineConfig {
    /// Defaults: socket `$XDG_RUNTIME_DIR/hece/engine.sock`, idle TTL 30 min, connect 20 s,
    /// base request timeout 120 s.
    pub fn default_for(model_dir: PathBuf) -> Result<Self, EngineClientError> {
        let runtime = std::env::var_os("XDG_RUNTIME_DIR")
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
        let sibling = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|d| d.join("hece-engine")))
            .filter(|p| p.is_file());
        Ok(sibling.unwrap_or_else(|| PathBuf::from("hece-engine")))
    }
}

impl Spawner for ProcessSpawner {
    fn spawn(&self, cfg: &EngineConfig) -> Result<Child, EngineClientError> {
        let program = Self::resolve(cfg)?;
        Command::new(program)
            .arg("serve")
            .arg("--socket")
            .arg(&cfg.socket)
            .arg("--idle-ttl-secs")
            .arg(cfg.idle_ttl.as_secs().to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
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
    conn: Option<UnixStream>,
    /// The worker this client started, if any; only that one is ever killed.
    child: Option<Child>,
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

async fn write_request(conn: &mut UnixStream, req: &Request) -> Result<(), EngineClientError> {
    let frame = encode_frame(req).map_err(|e| EngineClientError::Protocol(e.code().to_owned()))?;
    conn.write_all(&frame)
        .await
        .map_err(|_| EngineClientError::Crashed)
}

async fn read_response(conn: &mut UnixStream) -> Result<Response, EngineClientError> {
    let mut header = [0u8; 4];
    conn.read_exact(&mut header)
        .await
        .map_err(|_| EngineClientError::Crashed)?;
    let len = frame_len(header).map_err(|e| EngineClientError::Protocol(e.code().to_owned()))?;
    let mut body = vec![0u8; len];
    conn.read_exact(&mut body)
        .await
        .map_err(|_| EngineClientError::Crashed)?;
    decode_body(&body).map_err(|e| EngineClientError::Protocol(e.code().to_owned()))
}

async fn round_trip(conn: &mut UnixStream, req: &Request) -> Result<Response, EngineClientError> {
    write_request(conn, req).await?;
    read_response(conn).await
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
        let req = Request::Transcribe(TranscribeRequest {
            model_dir: self.cfg.model_dir.clone(),
            compute: self.cfg.compute.clone(),
            language: language.map(str::to_owned),
            vocabulary: vocabulary.cloned(),
            prompt_prefix: prompt_prefix.to_owned(),
            prompt_tokens,
            vad: true,
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

    /// Asks a running worker to exit. Does nothing if no worker is reachable.
    pub async fn shutdown(&self) -> Result<(), EngineClientError> {
        let result = self
            .call(&Request::Shutdown, self.cfg.base_timeout, false)
            .await;
        let mut state = self.state.lock().await;
        state.conn = None;
        state.child = None;
        match result? {
            Some(Response::Bye) | None => Ok(()),
            Some(other) => Err(unexpected(&other)),
        }
    }

    /// Sends one request on the (possibly new) connection. `Ok(None)` means no worker was
    /// reachable and spawning was not allowed. The connection is kept only after a clean answer.
    async fn call(
        &self,
        req: &Request,
        limit: Duration,
        allow_spawn: bool,
    ) -> Result<Option<Response>, EngineClientError> {
        let mut state = self.state.lock().await;
        let mut conn = if let Some(conn) = state.conn.take() {
            conn
        } else {
            match self.connect(&mut state, allow_spawn).await? {
                Some(conn) => conn,
                None => return Ok(None),
            }
        };
        match timeout(limit, round_trip(&mut conn, req)).await {
            Ok(Ok(Response::Error { code })) => {
                state.conn = Some(conn);
                Err(EngineClientError::Engine { code })
            }
            Ok(Ok(resp)) => {
                state.conn = Some(conn);
                Ok(Some(resp))
            }
            Ok(Err(e)) => {
                tracing::warn!(code = e.code(), "engine request failed");
                Err(e)
            }
            Err(_) => {
                tracing::warn!("engine request timed out");
                if let Some(child) = state.child.as_mut()
                    && let Err(e) = child.kill().await
                {
                    tracing::warn!(kind = ?e.kind(), "failed to kill timed-out engine");
                }
                state.child = None;
                Err(EngineClientError::Timeout)
            }
        }
    }

    /// Connects to the socket, spawning the worker if allowed, then performs the handshake.
    async fn connect(
        &self,
        state: &mut State,
        allow_spawn: bool,
    ) -> Result<Option<UnixStream>, EngineClientError> {
        let deadline = Instant::now() + self.cfg.connect_timeout;
        let mut spawned = false;
        let mut stream = loop {
            if let Ok(stream) = UnixStream::connect(&self.cfg.socket).await {
                break stream;
            }
            if !allow_spawn {
                return Ok(None);
            }
            if spawned {
                Self::check_child(state)?;
            } else {
                state.child = Some(self.spawner.spawn(&self.cfg)?);
                spawned = true;
                tracing::debug!("spawned engine worker");
                continue;
            }
            if Instant::now() >= deadline {
                return Err(EngineClientError::ConnectTimeout);
            }
            sleep(RETRY_INTERVAL).await;
        };
        let hello = timeout(
            self.cfg.connect_timeout,
            round_trip(&mut stream, &Request::Hello),
        )
        .await
        .map_err(|_| EngineClientError::ConnectTimeout)??;
        match hello {
            Response::Hello { protocol, .. } if protocol == PROTOCOL_VERSION => Ok(Some(stream)),
            Response::Hello { protocol, .. } => {
                Err(EngineClientError::ProtocolMismatch { worker: protocol })
            }
            other => Err(unexpected(&other)),
        }
    }

    /// Notices a freshly spawned worker that died before binding the socket.
    fn check_child(state: &mut State) -> Result<(), EngineClientError> {
        let Some(child) = state.child.as_mut() else {
            return Ok(());
        };
        match child.try_wait() {
            Ok(Some(status)) if status.code() == Some(EXIT_ALREADY_RUNNING) => {
                // Another worker owns the socket; keep retrying the connect.
                state.child = None;
                Ok(())
            }
            Ok(Some(status)) => {
                state.child = None;
                Err(EngineClientError::Spawn(format!(
                    "worker exited with code {:?}",
                    status.code()
                )))
            }
            Ok(None) | Err(_) => Ok(()),
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
