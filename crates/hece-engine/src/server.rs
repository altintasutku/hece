//! The worker server: a [`Backend`] behind the worker socket (a named pipe on Windows) speaking
//! the framed JSON protocol of `hece_core::ipc`. One connection is served at a time; the worker
//! exits after an idle period.
//!
//! Logging rule: requests are never logged (they hold vocabulary terms and audio) and IPC
//! errors are logged by code and length only, because codec messages can echo input.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::io::{ErrorKind, Read, Write};
use std::net::Shutdown;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::PoisonError;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use hece_core::Transcript;
use hece_core::ipc::{
    CONN_IDLE_TIMEOUT, IpcError, PROTOCOL_VERSION, Request, Response, TranscribeRequest,
    decode_body, encode_frame, frame_len,
};

use crate::audio::SAMPLE_RATE;
use crate::engine::ComputeKind;
use crate::error::EngineError;
use crate::socket::{Conn, Listener, accept, prepare_listener};

/// How long a connection may stay silent between frames before it is closed.
const READ_TIMEOUT: Duration = CONN_IDLE_TIMEOUT;
/// How long one accept poll waits while idle.
const POLL_INTERVAL: Duration = Duration::from_millis(200);
/// How often the watchdog checks its deadline.
const WATCHDOG_POLL: Duration = Duration::from_millis(100);
/// Default idle time before the worker exits.
pub const DEFAULT_IDLE_TTL: Duration = Duration::from_mins(30);
/// Most bytes read from a client after rejecting an oversized frame.
const DRAIN_LIMIT: usize = 128 * 1024 * 1024;
/// Longest time spent draining such a client.
const DRAIN_TIME: Duration = Duration::from_secs(2);
/// Fixed part of the per-request watchdog deadline.
pub const WATCHDOG_BASE: Duration = Duration::from_secs(300);
/// Exit code of a worker killed by its own watchdog.
pub const EXIT_WATCHDOG: i32 = 5;
/// Rendered prompts kept before the cache is emptied.
const PROMPT_CACHE_LIMIT: usize = 64;

/// A speech-to-text engine the server can drive.
pub trait Backend {
    /// Loads `model_dir` (replacing any other loaded model).
    fn load(&mut self, model_dir: &Path, compute: ComputeKind) -> Result<(), EngineError>;

    /// The model currently loaded, if any.
    fn loaded(&self) -> Option<&Path>;

    /// Transcribes `audio`, first loading (or reloading on a different `model_dir`/`compute`)
    /// the model named by `req`. The vocabulary prompt is rendered with the model's tokenizer
    /// and cached by [`PromptCache`].
    fn transcribe(
        &mut self,
        req: &TranscribeRequest,
        audio: &[i16],
    ) -> Result<Transcript, EngineError>;
}

/// Cache of rendered vocabulary prompts. The render closure is injected so the cache is pure.
#[derive(Debug, Default)]
pub struct PromptCache {
    entries: HashMap<u64, Option<String>>,
}

impl PromptCache {
    /// Cache key of one prompt rendering.
    #[must_use]
    pub fn key(model_dir: &Path, prefix: &str, prompt_tokens: usize, terms: &[String]) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        model_dir.hash(&mut h);
        prefix.hash(&mut h);
        prompt_tokens.hash(&mut h);
        terms.hash(&mut h);
        h.finish()
    }

    /// Returns the cached prompt for `key` or renders and stores it. Failed renders are not cached.
    pub fn get_or_render(
        &mut self,
        key: u64,
        render: impl FnOnce() -> Result<Option<String>, EngineError>,
    ) -> Result<Option<String>, EngineError> {
        if let Some(hit) = self.entries.get(&key) {
            return Ok(hit.clone());
        }
        let rendered = render()?;
        if self.entries.len() >= PROMPT_CACHE_LIMIT {
            self.entries.clear();
        }
        self.entries.insert(key, rendered.clone());
        Ok(rendered)
    }

    /// Drops every entry; call when the model changes.
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

/// Backend on the local `CTranslate2` engine (CUDA).
#[cfg(feature = "cuda")]
#[derive(Debug, Default)]
pub struct CudaBackend {
    engine: Option<crate::LocalEngine>,
    current: Option<(PathBuf, ComputeKind)>,
    cache: PromptCache,
}

#[cfg(feature = "cuda")]
impl CudaBackend {
    /// An empty backend; the model loads on the first `load` or `transcribe`.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn ensure_loaded(&mut self, model_dir: &Path, compute: ComputeKind) -> Result<(), EngineError> {
        let same = self
            .current
            .as_ref()
            .is_some_and(|(dir, c)| dir == model_dir && *c == compute);
        if same && self.engine.is_some() {
            return Ok(());
        }
        // Free the old model before loading the new one so both never sit in VRAM together.
        self.engine = None;
        self.current = None;
        self.cache.clear();
        let mut engine = crate::LocalEngine::load(model_dir, compute, 0)?;
        engine.warm_up()?;
        self.engine = Some(engine);
        self.current = Some((model_dir.to_path_buf(), compute));
        Ok(())
    }
}

#[cfg(feature = "cuda")]
impl Backend for CudaBackend {
    fn load(&mut self, model_dir: &Path, compute: ComputeKind) -> Result<(), EngineError> {
        self.ensure_loaded(model_dir, compute)
    }

    fn loaded(&self) -> Option<&Path> {
        self.current.as_ref().map(|(dir, _)| dir.as_path())
    }

    fn transcribe(
        &mut self,
        req: &TranscribeRequest,
        audio: &[i16],
    ) -> Result<Transcript, EngineError> {
        let compute =
            req.compute
                .parse::<ComputeKind>()
                .map_err(|e| EngineError::ModelDirInvalid {
                    reason: e.to_string(),
                })?;
        self.ensure_loaded(&req.model_dir, compute)?;
        let Some(engine) = self.engine.as_mut() else {
            return Err(EngineError::Decode("engine not loaded".to_owned()));
        };
        let initial_prompt = match &req.vocabulary {
            Some(vocab) => {
                let key = PromptCache::key(
                    &req.model_dir,
                    &req.prompt_prefix,
                    req.prompt_tokens,
                    &vocab.terms,
                );
                self.cache.get_or_render(key, || {
                    engine.vocabulary_prompt(vocab, &req.prompt_prefix, req.prompt_tokens)
                })?
            }
            None => None,
        };
        let opts = crate::TranscribeOptions {
            language: req.language.clone(),
            initial_prompt,
            vad: req.vad,
            decode: crate::decode::DecodeOptions {
                beam_size: req.beam_size.map_or(5, |b| b.clamp(1, 10) as usize),
                ..crate::decode::DecodeOptions::default()
            },
            ..crate::TranscribeOptions::default()
        };
        engine.transcribe(audio, &opts)
    }
}

/// Command line of `hece-engine serve`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServeArgs {
    /// Socket path; `None` means [`crate::socket::socket_path`].
    pub socket: Option<PathBuf>,
    /// Idle time before the worker exits.
    pub idle_ttl: Duration,
}

/// A malformed `serve` command line.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("usage: hece-engine serve [--socket <path>] [--idle-ttl-secs <n>]")]
pub struct ArgsError;

impl ServeArgs {
    /// Parses the arguments after `serve`.
    pub fn parse(args: &[String]) -> Result<Self, ArgsError> {
        let mut socket = None;
        let mut idle_ttl = DEFAULT_IDLE_TTL;
        let mut it = args.iter();
        while let Some(flag) = it.next() {
            let value = it.next().ok_or(ArgsError)?;
            match flag.as_str() {
                "--socket" => socket = Some(PathBuf::from(value)),
                "--idle-ttl-secs" => {
                    let secs: u64 = value.parse().map_err(|_| ArgsError)?;
                    if secs == 0 {
                        return Err(ArgsError);
                    }
                    idle_ttl = Duration::from_secs(secs);
                }
                _ => return Err(ArgsError),
            }
        }
        Ok(Self { socket, idle_ttl })
    }
}

/// What to do after answering a request.
enum Next {
    Continue,
    Shutdown,
}

/// Why a connection stopped being served.
enum Closed {
    /// Peer hung up, timed out while idle, or sent something unusable.
    Done,
    /// The client asked the worker to exit.
    Shutdown,
}

/// Whether a request deadline has passed.
#[must_use]
pub fn expired(deadline: Option<Instant>, now: Instant) -> bool {
    deadline.is_some_and(|d| now >= d)
}

/// Settings of [`serve_with`].
pub struct ServeOptions {
    /// Idle time before the worker exits.
    pub idle_ttl: Duration,
    /// Watchdog allowance per request before the audio-length part (default [`WATCHDOG_BASE`]).
    pub watchdog_base: Duration,
    /// Called when a request outlives its deadline; the default exits with [`EXIT_WATCHDOG`].
    pub on_expire: Box<dyn Fn() + Send + Sync>,
}

impl ServeOptions {
    /// Defaults with the given idle TTL: a 300 s watchdog base that exits the process.
    #[must_use]
    pub fn new(idle_ttl: Duration) -> Self {
        Self {
            idle_ttl,
            watchdog_base: WATCHDOG_BASE,
            on_expire: Box::new(|| std::process::exit(EXIT_WATCHDOG)),
        }
    }
}

impl std::fmt::Debug for ServeOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServeOptions")
            .field("idle_ttl", &self.idle_ttl)
            .field("watchdog_base", &self.watchdog_base)
            .finish_non_exhaustive()
    }
}

/// Deadline of the request being served, shared with the watchdog thread.
type Deadline = Mutex<Option<Instant>>;

fn set_deadline(deadline: &Deadline, value: Option<Instant>) {
    *deadline.lock().unwrap_or_else(PoisonError::into_inner) = value;
}

/// Watchdog allowance for `request`: base plus two seconds per second of audio. Requests that
/// do no engine work have none.
fn budget(request: &Request, base: Duration) -> Option<Duration> {
    match request {
        Request::Load(_) => Some(base),
        Request::Transcribe(req) => {
            // Decoded bytes are 3/4 of the base64 length; two bytes per sample.
            let samples = req.audio.0.len() / 4 * 3 / 2;
            let audio_s = u64::try_from(samples / SAMPLE_RATE).unwrap_or(u64::MAX);
            Some(base.saturating_add(Duration::from_secs(audio_s.saturating_mul(2))))
        }
        Request::Hello | Request::Shutdown => None,
    }
}

/// Fires `on_expire` once if the shared deadline passes while a request still runs.
fn watchdog_loop(deadline: &Deadline, done: &AtomicBool, on_expire: &(dyn Fn() + Send + Sync)) {
    while !done.load(Ordering::Acquire) {
        std::thread::park_timeout(WATCHDOG_POLL);
        let current = *deadline.lock().unwrap_or_else(PoisonError::into_inner);
        if expired(current, Instant::now()) {
            tracing::error!("engine.watchdog");
            set_deadline(deadline, None);
            on_expire();
        }
    }
}

/// [`serve_with`] with default options, taking ownership of the listener.
#[expect(
    clippy::needless_pass_by_value,
    reason = "owning the listener closes the socket when serve returns"
)]
pub fn serve<B: Backend>(
    listener: Listener,
    backend: &mut B,
    idle_ttl: Duration,
) -> Result<(), EngineError> {
    serve_with(&listener, backend, &ServeOptions::new(idle_ttl))
}

/// Accepts connections until the worker has been idle for `opts.idle_ttl` or a client sends
/// `Shutdown`. The idle clock restarts after every response and never fires during a request.
/// A watchdog thread calls `opts.on_expire` if a request outlives its deadline.
pub fn serve_with<B: Backend>(
    listener: &Listener,
    backend: &mut B,
    opts: &ServeOptions,
) -> Result<(), EngineError> {
    prepare_listener(listener).map_err(|e| EngineError::Socket(e.to_string()))?;
    let deadline: Deadline = Mutex::new(None);
    let done = AtomicBool::new(false);
    std::thread::scope(|scope| {
        let watchdog = scope.spawn(|| watchdog_loop(&deadline, &done, &*opts.on_expire));
        let watch = Watch {
            deadline: &deadline,
            base: opts.watchdog_base,
        };
        let result = accept_loop(listener, backend, opts.idle_ttl, &watch);
        done.store(true, Ordering::Release);
        watchdog.thread().unpark();
        result
    })
}

/// The watchdog state a connection handler updates around each request.
struct Watch<'a> {
    deadline: &'a Deadline,
    base: Duration,
}

fn accept_loop<B: Backend>(
    listener: &Listener,
    backend: &mut B,
    idle_ttl: Duration,
    watch: &Watch<'_>,
) -> Result<(), EngineError> {
    let mut last_activity = Instant::now();
    loop {
        match accept(listener, POLL_INTERVAL) {
            Ok(Some(stream)) => {
                if let Closed::Shutdown =
                    handle_connection(stream, backend, &mut last_activity, watch)
                {
                    return Ok(());
                }
                continue;
            }
            Ok(None) => {}
            Err(e) => {
                tracing::warn!(kind = ?e.kind(), "engine.accept_failed");
                std::thread::sleep(POLL_INTERVAL);
            }
        }
        if last_activity.elapsed() >= idle_ttl {
            tracing::info!("engine.idle_exit");
            return Ok(());
        }
    }
}

fn handle_connection<B: Backend>(
    mut stream: Conn,
    backend: &mut B,
    last_activity: &mut Instant,
    watch: &Watch<'_>,
) -> Closed {
    if stream.set_nonblocking(false).is_err()
        || stream.set_read_timeout(Some(READ_TIMEOUT)).is_err()
        || stream.set_write_timeout(Some(READ_TIMEOUT)).is_err()
    {
        return Closed::Done;
    }
    loop {
        let request = match read_request(&mut stream) {
            Ok(Some(request)) => request,
            Ok(None) => return Closed::Done,
            Err(e) => {
                tracing::warn!(code = e.code(), "engine.bad_frame");
                // Best effort: the peer may already be gone.
                let _ = write_response(&mut stream, &error_response(e.code()));
                if let IpcError::FrameTooLarge(announced) = e {
                    drain_and_close(&mut stream, announced);
                }
                return Closed::Done;
            }
        };
        set_deadline(
            watch.deadline,
            budget(&request, watch.base).map(|b| Instant::now() + b),
        );
        let (response, next) = handle(request, backend);
        set_deadline(watch.deadline, None);
        if write_response(&mut stream, &response).is_err() {
            return Closed::Done;
        }
        *last_activity = Instant::now();
        if let Next::Shutdown = next {
            return Closed::Shutdown;
        }
    }
}

/// Lets a client that is still sending an oversized frame finish, so it reads our error
/// instead of a connection reset. Bounded in bytes and time.
fn drain_and_close(stream: &mut Conn, announced: usize) {
    let _ = stream.shutdown(Shutdown::Write);
    let deadline = Instant::now() + DRAIN_TIME;
    let mut remaining = announced.min(DRAIN_LIMIT);
    let mut buf = vec![0u8; 64 * 1024];
    while remaining > 0 {
        let Some(left) = deadline.checked_duration_since(Instant::now()) else {
            break;
        };
        if stream.set_read_timeout(Some(left)).is_err() {
            break;
        }
        let want = remaining.min(buf.len());
        match stream.read(&mut buf[..want]) {
            Ok(0) | Err(_) => break,
            Ok(n) => remaining -= n,
        }
    }
}

/// Reads one request. `Ok(None)` is a clean end of the connection (EOF or idle timeout).
fn read_request(stream: &mut Conn) -> Result<Option<Request>, IpcError> {
    let mut header = [0u8; 4];
    if let Err(e) = stream.read_exact(&mut header) {
        // EOF and idle timeout end the connection quietly; anything else is worth a trace.
        if !matches!(
            e.kind(),
            ErrorKind::UnexpectedEof | ErrorKind::WouldBlock | ErrorKind::TimedOut
        ) {
            tracing::debug!(kind = ?e.kind(), "engine.header_read_failed");
        }
        return Ok(None);
    }
    let len = frame_len(header)?;
    // `take` + `read_to_end` grows with the data actually sent instead of trusting the header.
    let mut body = Vec::new();
    let read = stream.take(len as u64).read_to_end(&mut body);
    if let Err(e) = &read {
        tracing::debug!(kind = ?e.kind(), announced = len, got = body.len(), "engine.body_read_failed");
        return Ok(None);
    }
    if body.len() != len {
        tracing::debug!(announced = len, got = body.len(), "engine.short_body");
        return Ok(None);
    }
    decode_body(&body).map(Some)
}

fn write_response(stream: &mut Conn, response: &Response) -> Result<(), ()> {
    let frame = encode_frame(response).map_err(|e| {
        tracing::error!(code = e.code(), "engine.encode_failed");
    })?;
    stream.write_all(&frame).map_err(|e| {
        tracing::debug!(kind = ?e.kind(), "engine.write_failed");
    })
}

fn error_response(code: &str) -> Response {
    Response::Error {
        code: code.to_owned(),
    }
}

/// Logs a backend failure (code and library message, never audio or text) and builds the reply.
fn engine_error(e: &EngineError) -> Response {
    tracing::error!(code = e.code(), detail = %e, "engine.error");
    error_response(e.code())
}

fn parse_compute(raw: &str) -> Result<ComputeKind, Response> {
    raw.parse().map_err(|_| error_response("bad_compute"))
}

fn handle<B: Backend>(request: Request, backend: &mut B) -> (Response, Next) {
    match request {
        Request::Hello => (
            Response::Hello {
                protocol: PROTOCOL_VERSION,
                engine_version: env!("CARGO_PKG_VERSION").to_owned(),
                loaded_model: backend.loaded().map(Path::to_path_buf),
            },
            Next::Continue,
        ),
        Request::Load(load) => {
            let response = match parse_compute(&load.compute) {
                Err(response) => response,
                Ok(compute) => {
                    let started = Instant::now();
                    match backend.load(&load.model_dir, compute) {
                        Ok(()) => {
                            let load_ms = millis(started.elapsed());
                            tracing::info!(
                                model = load.model_dir.file_name().and_then(|n| n.to_str()),
                                load_ms,
                                "engine.loaded"
                            );
                            Response::Loaded { load_ms }
                        }
                        Err(e) => engine_error(&e),
                    }
                }
            };
            (response, Next::Continue)
        }
        Request::Transcribe(req) => (transcribe(&req, backend), Next::Continue),
        Request::Shutdown => (Response::Bye, Next::Shutdown),
    }
}

fn transcribe<B: Backend>(req: &TranscribeRequest, backend: &mut B) -> Response {
    if let Err(response) = parse_compute(&req.compute) {
        return response;
    }
    let audio = match req.audio.decode() {
        Ok(audio) => audio,
        Err(e) => {
            tracing::warn!(code = e.code(), "engine.bad_audio");
            return error_response(e.code());
        }
    };
    let started = Instant::now();
    match backend.transcribe(req, &audio) {
        Ok(transcript) => {
            #[expect(
                clippy::cast_precision_loss,
                reason = "a log field; sample counts are far below f64 precision limits"
            )]
            let audio_s = audio.len() as f64 / SAMPLE_RATE as f64;
            tracing::info!(
                audio_s,
                elapsed_ms = millis(started.elapsed()),
                segments = transcript.segments.as_ref().map_or(0, Vec::len),
                "engine.transcribed"
            );
            Response::Transcript(transcript)
        }
        Err(e) => engine_error(&e),
    }
}

fn millis(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn prompt_cache_hit_does_not_render_twice() {
        let calls = Cell::new(0);
        let render = || {
            calls.set(calls.get() + 1);
            Ok(Some("Vocabulary: alpha".to_owned()))
        };
        let mut cache = PromptCache::default();
        let terms = vec!["alpha".to_owned()];
        let key = PromptCache::key(Path::new("/m"), "Vocabulary: ", 64, &terms);
        let a = cache.get_or_render(key, render).unwrap();
        let b = cache.get_or_render(key, render).unwrap();
        assert_eq!(a, b);
        assert_eq!(calls.get(), 1);
        // A different term list is a different key; clearing forces a re-render.
        let other = PromptCache::key(Path::new("/m"), "Vocabulary: ", 64, &["beta".to_owned()]);
        assert_ne!(key, other);
        cache.clear();
        cache.get_or_render(key, render).unwrap();
        assert_eq!(calls.get(), 2);
    }

    #[test]
    fn prompt_cache_does_not_store_failures() {
        let mut cache = PromptCache::default();
        let failed = cache.get_or_render(1, || Err(EngineError::Tokenizer("x".to_owned())));
        assert!(failed.is_err());
        let ok = cache.get_or_render(1, || Ok(None)).unwrap();
        assert_eq!(ok, None);
    }
}
