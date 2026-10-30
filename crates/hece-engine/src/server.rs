//! The worker server: a [`Backend`] behind a unix socket speaking the framed JSON protocol of
//! `hece_core::ipc`. One connection is served at a time; the worker exits after an idle period.
//!
//! Logging rule: requests are never logged (they hold vocabulary terms and audio) and IPC
//! errors are logged by code and length only, because codec messages can echo input.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::io::{ErrorKind, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use hece_core::Transcript;
use hece_core::ipc::{
    IpcError, PROTOCOL_VERSION, Request, Response, TranscribeRequest, decode_body, encode_frame,
    frame_len,
};

use crate::audio::SAMPLE_RATE;
use crate::engine::ComputeKind;
use crate::error::EngineError;

/// How long a connection may stay silent between frames before it is closed.
const READ_TIMEOUT: Duration = Duration::from_secs(30);
/// Pause between accept polls while idle.
const POLL_INTERVAL: Duration = Duration::from_millis(200);
/// Default idle time before the worker exits.
pub const DEFAULT_IDLE_TTL: Duration = Duration::from_mins(15);
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
                    idle_ttl = Duration::from_secs(value.parse().map_err(|_| ArgsError)?);
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

/// Accepts connections until the worker has been idle for `idle_ttl` or a client sends
/// `Shutdown`. The idle clock restarts after every response and never fires during a request.
#[expect(
    clippy::needless_pass_by_value,
    reason = "owning the listener closes the socket when serve returns"
)]
pub fn serve<B: Backend>(
    listener: UnixListener,
    backend: &mut B,
    idle_ttl: Duration,
) -> Result<(), EngineError> {
    listener
        .set_nonblocking(true)
        .map_err(|e| EngineError::Socket(e.to_string()))?;
    let mut last_activity = Instant::now();
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                if let Closed::Shutdown = handle_connection(stream, backend, &mut last_activity) {
                    return Ok(());
                }
                last_activity = Instant::now();
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => {
                if last_activity.elapsed() >= idle_ttl {
                    tracing::info!("engine.idle_exit");
                    return Ok(());
                }
                std::thread::sleep(POLL_INTERVAL);
            }
            Err(e) => {
                tracing::warn!(kind = ?e.kind(), "engine.accept_failed");
                std::thread::sleep(POLL_INTERVAL);
            }
        }
    }
}

fn handle_connection<B: Backend>(
    mut stream: UnixStream,
    backend: &mut B,
    last_activity: &mut Instant,
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
                return Closed::Done;
            }
        };
        let (response, next) = handle(request, backend);
        if write_response(&mut stream, &response).is_err() {
            return Closed::Done;
        }
        *last_activity = Instant::now();
        if let Next::Shutdown = next {
            return Closed::Shutdown;
        }
    }
}

/// Reads one request. `Ok(None)` is a clean end of the connection (EOF or idle timeout).
fn read_request(stream: &mut UnixStream) -> Result<Option<Request>, IpcError> {
    let mut header = [0u8; 4];
    match stream.read_exact(&mut header) {
        Ok(()) => {}
        Err(e)
            if matches!(
                e.kind(),
                ErrorKind::UnexpectedEof | ErrorKind::WouldBlock | ErrorKind::TimedOut
            ) =>
        {
            return Ok(None);
        }
        Err(_) => return Ok(None),
    }
    let len = frame_len(header)?;
    // `take` + `read_to_end` grows with the data actually sent instead of trusting the header.
    let mut body = Vec::new();
    let read = stream.take(len as u64).read_to_end(&mut body);
    if read.is_err() || body.len() != len {
        return Ok(None);
    }
    decode_body(&body).map(Some)
}

fn write_response(stream: &mut UnixStream, response: &Response) -> Result<(), ()> {
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
                                model_dir = %load.model_dir.display(),
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
