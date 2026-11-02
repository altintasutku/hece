//! Worker server over real unix sockets (named pipes on Windows) with a fake backend. No CUDA
//! needed.

#![expect(
    clippy::unwrap_used,
    reason = "clippy.toml only exempts #[test] functions, not helper functions in test files"
)]

use std::io::{Read, Write};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use hece_core::Transcript;
use hece_core::ipc::{
    AudioB64, LoadRequest, PROTOCOL_VERSION, Request, Response, TranscribeRequest, decode_body,
    encode_frame,
};
use hece_engine::engine::ComputeKind;
use hece_engine::error::EngineError;
use hece_engine::server::{Backend, ServeArgs, ServeOptions, expired, serve, serve_with};
use hece_engine::socket::{BindError, Bound, bind, socket_path_from};

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// Unique directory under the system temp dir, removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("hece-t-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // bind() rejects group/world accessible directories.
        #[cfg(unix)]
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        Self(dir)
    }

    fn sock(&self) -> PathBuf {
        self.0.join("s.sock")
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[derive(Default)]
struct Fake {
    loaded: Option<PathBuf>,
    seen: Arc<Mutex<Vec<Vec<i16>>>>,
    delay: Duration,
    fail: bool,
}

impl Backend for Fake {
    fn load(&mut self, model_dir: &Path, _compute: ComputeKind) -> Result<(), EngineError> {
        self.loaded = Some(model_dir.to_path_buf());
        Ok(())
    }

    fn loaded(&self) -> Option<&Path> {
        self.loaded.as_deref()
    }

    fn transcribe(
        &mut self,
        req: &TranscribeRequest,
        audio: &[i16],
    ) -> Result<Transcript, EngineError> {
        thread::sleep(self.delay);
        if self.fail {
            return Err(EngineError::Decode("boom".to_owned()));
        }
        self.loaded = Some(req.model_dir.clone());
        self.seen.lock().unwrap().push(audio.to_vec());
        Ok(Transcript {
            text: "ok".to_owned(),
            segments: None,
            language: "en".to_owned(),
            provider: "fake".to_owned(),
            low_confidence: false,
        })
    }
}

type ServeHandle = JoinHandle<(Result<(), EngineError>, Instant)>;

fn start(dir: &TempDir, fake: Fake, ttl: Duration) -> ServeHandle {
    let Bound { listener, lock } = bind(&dir.sock()).unwrap();
    thread::spawn(move || {
        let _lock = lock;
        let mut fake = fake;
        let res = serve(listener, &mut fake, ttl);
        (res, Instant::now())
    })
}

/// Client end of a connection.
#[cfg(unix)]
type Client = UnixStream;
/// Client end of a connection: the named pipe opened as a file.
#[cfg(windows)]
type Client = std::fs::File;

#[cfg(unix)]
fn connect(dir: &TempDir) -> Client {
    let s = UnixStream::connect(dir.sock()).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    s
}

/// Retries while every pipe instance is busy (`ERROR_PIPE_BUSY`), as real clients do.
#[cfg(windows)]
fn connect(dir: &TempDir) -> Client {
    const ERROR_PIPE_BUSY: i32 = 231;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(hece_core::ipc::pipe_name(&dir.sock()))
        {
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) && Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(10));
            }
            other => return other.unwrap(),
        }
    }
}

fn send(s: &mut Client, req: &Request) {
    s.write_all(&encode_frame(req).unwrap()).unwrap();
}

fn recv(s: &mut Client) -> Response {
    let mut header = [0u8; 4];
    s.read_exact(&mut header).unwrap();
    let mut body = vec![0u8; u32::from_be_bytes(header) as usize];
    s.read_exact(&mut body).unwrap();
    decode_body(&body).unwrap()
}

fn transcribe_req(samples: &[i16]) -> Request {
    Request::Transcribe(TranscribeRequest {
        model_dir: PathBuf::from("/models/m"),
        compute: "int8_float16".to_owned(),
        language: Some("en".to_owned()),
        vocabulary: None,
        prompt_prefix: String::new(),
        prompt_tokens: 0,
        vad: true,
        beam_size: None,
        audio: AudioB64::encode(samples),
    })
}

const LONG: Duration = Duration::from_mins(1);

fn shutdown(dir: &TempDir, handle: ServeHandle) {
    let mut s = connect(dir);
    send(&mut s, &Request::Shutdown);
    assert_eq!(recv(&mut s), Response::Bye);
    assert!(handle.join().unwrap().0.is_ok());
}

#[test]
fn hello_reports_protocol_and_loaded_model() {
    let dir = TempDir::new();
    let handle = start(&dir, Fake::default(), LONG);
    let mut s = connect(&dir);
    send(&mut s, &Request::Hello);
    match recv(&mut s) {
        Response::Hello {
            protocol,
            engine_version,
            loaded_model,
        } => {
            assert_eq!(protocol, PROTOCOL_VERSION);
            assert_eq!(engine_version, env!("CARGO_PKG_VERSION"));
            assert_eq!(loaded_model, None);
        }
        other => panic!("unexpected {other:?}"),
    }
    drop(s);
    shutdown(&dir, handle);
}

#[test]
fn load_then_hello_shows_model() {
    let dir = TempDir::new();
    let handle = start(&dir, Fake::default(), LONG);
    let mut s = connect(&dir);
    send(
        &mut s,
        &Request::Load(LoadRequest {
            model_dir: PathBuf::from("/models/m"),
            compute: "float16".to_owned(),
        }),
    );
    assert!(matches!(recv(&mut s), Response::Loaded { .. }));
    send(&mut s, &Request::Hello);
    match recv(&mut s) {
        Response::Hello { loaded_model, .. } => {
            assert_eq!(loaded_model, Some(PathBuf::from("/models/m")));
        }
        other => panic!("unexpected {other:?}"),
    }
    send(
        &mut s,
        &Request::Load(LoadRequest {
            model_dir: PathBuf::from("/models/m"),
            compute: "nope".to_owned(),
        }),
    );
    assert_eq!(
        recv(&mut s),
        Response::Error {
            code: "bad_compute".to_owned()
        }
    );
    drop(s);
    shutdown(&dir, handle);
}

#[test]
fn transcribe_decodes_audio_for_the_backend() {
    let dir = TempDir::new();
    let fake = Fake::default();
    let seen = Arc::clone(&fake.seen);
    let handle = start(&dir, fake, LONG);
    let mut s = connect(&dir);
    send(&mut s, &transcribe_req(&[1, -2, 3]));
    match recv(&mut s) {
        Response::Transcript(t) => assert_eq!(t.text, "ok"),
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(*seen.lock().unwrap(), vec![vec![1, -2, 3]]);
    drop(s);
    shutdown(&dir, handle);
}

#[test]
fn bad_audio_is_an_error_response() {
    let dir = TempDir::new();
    let handle = start(&dir, Fake::default(), LONG);
    let mut s = connect(&dir);
    let Request::Transcribe(mut req) = transcribe_req(&[1]) else {
        unreachable!()
    };
    req.audio = AudioB64("!!!".to_owned());
    send(&mut s, &Request::Transcribe(req));
    assert_eq!(
        recv(&mut s),
        Response::Error {
            code: "bad_audio".to_owned()
        }
    );
    drop(s);
    shutdown(&dir, handle);
}

#[test]
fn backend_error_carries_its_code() {
    let dir = TempDir::new();
    let fake = Fake {
        fail: true,
        ..Fake::default()
    };
    let handle = start(&dir, fake, LONG);
    let mut s = connect(&dir);
    send(&mut s, &transcribe_req(&[1]));
    assert_eq!(
        recv(&mut s),
        Response::Error {
            code: "decode_failed".to_owned()
        }
    );
    // The connection survives a backend error.
    send(&mut s, &Request::Hello);
    assert!(matches!(recv(&mut s), Response::Hello { .. }));
    drop(s);
    shutdown(&dir, handle);
}

#[test]
fn oversized_frame_is_rejected_and_server_survives() {
    let dir = TempDir::new();
    let handle = start(&dir, Fake::default(), LONG);
    let mut s = connect(&dir);
    let announced = u32::try_from(65 * 1024 * 1024).unwrap();
    s.write_all(&announced.to_be_bytes()).unwrap();
    assert_eq!(
        recv(&mut s),
        Response::Error {
            code: "frame_too_large".to_owned()
        }
    );
    drop(s);
    let mut s2 = connect(&dir);
    send(&mut s2, &Request::Hello);
    assert!(matches!(recv(&mut s2), Response::Hello { .. }));
    drop(s2);
    shutdown(&dir, handle);
}

#[test]
fn oversized_frame_with_a_real_body_still_gets_the_error() {
    let dir = TempDir::new();
    let handle = start(&dir, Fake::default(), LONG);
    let mut s = connect(&dir);
    #[cfg(unix)]
    s.set_write_timeout(Some(Duration::from_secs(10))).unwrap();
    let total = 70 * 1024 * 1024;
    let mut writer = s.try_clone().unwrap();
    let sender = thread::spawn(move || {
        let header = u32::try_from(total).unwrap().to_be_bytes();
        writer.write_all(&header).unwrap();
        let chunk = vec![b'x'; 1024 * 1024];
        for _ in 0..70 {
            writer.write_all(&chunk).unwrap();
        }
    });
    sender.join().unwrap();
    assert_eq!(
        recv(&mut s),
        Response::Error {
            code: "frame_too_large".to_owned()
        }
    );
    drop(s);
    shutdown(&dir, handle);
}

#[cfg(unix)]
#[test]
fn truncated_body_closes_the_connection_and_server_survives() {
    let dir = TempDir::new();
    let handle = start(&dir, Fake::default(), LONG);
    let mut s = connect(&dir);
    s.write_all(&100u32.to_be_bytes()).unwrap();
    s.write_all(b"0123456789").unwrap();
    s.shutdown(std::net::Shutdown::Write).unwrap();
    let mut rest = Vec::new();
    s.read_to_end(&mut rest).unwrap();
    assert_eq!(rest, [] as [u8; 0]);
    let mut s2 = connect(&dir);
    send(&mut s2, &Request::Hello);
    assert!(matches!(recv(&mut s2), Response::Hello { .. }));
    drop(s2);
    shutdown(&dir, handle);
}

/// Named pipes cannot be half closed, so the client hangs up mid-body instead.
#[cfg(windows)]
#[test]
fn hangup_mid_body_closes_the_connection_and_server_survives() {
    let dir = TempDir::new();
    let handle = start(&dir, Fake::default(), LONG);
    let mut s = connect(&dir);
    s.write_all(&100u32.to_be_bytes()).unwrap();
    s.write_all(b"0123456789").unwrap();
    drop(s);
    let mut s2 = connect(&dir);
    send(&mut s2, &Request::Hello);
    assert!(matches!(recv(&mut s2), Response::Hello { .. }));
    drop(s2);
    shutdown(&dir, handle);
}

#[test]
fn transcribe_with_unknown_compute_is_bad_compute() {
    let dir = TempDir::new();
    let handle = start(&dir, Fake::default(), LONG);
    let mut s = connect(&dir);
    let Request::Transcribe(mut req) = transcribe_req(&[1]) else {
        unreachable!()
    };
    req.compute = "bf16".to_owned();
    send(&mut s, &Request::Transcribe(req));
    assert_eq!(
        recv(&mut s),
        Response::Error {
            code: "bad_compute".to_owned()
        }
    );
    drop(s);
    shutdown(&dir, handle);
}

#[test]
fn socket_path_needs_a_runtime_dir() {
    assert_eq!(socket_path_from(None).unwrap_err().code(), "no_runtime_dir");
    assert_eq!(
        socket_path_from(Some(std::ffi::OsString::new()))
            .unwrap_err()
            .code(),
        "no_runtime_dir"
    );
    assert_eq!(
        socket_path_from(Some("/run/user/1".into())).unwrap(),
        PathBuf::from("/run/user/1")
            .join("hece")
            .join("engine.sock")
    );
}

#[cfg(unix)]
#[test]
fn bind_rejects_an_insecure_existing_directory() {
    let dir = TempDir::new();
    std::fs::set_permissions(&dir.0, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(matches!(bind(&dir.sock()), Err(BindError::Io(_))));
}

#[test]
fn invalid_json_is_rejected_and_server_survives() {
    let dir = TempDir::new();
    let handle = start(&dir, Fake::default(), LONG);
    let mut s = connect(&dir);
    let body = b"{not json";
    s.write_all(&u32::try_from(body.len()).unwrap().to_be_bytes())
        .unwrap();
    s.write_all(body).unwrap();
    assert_eq!(
        recv(&mut s),
        Response::Error {
            code: "bad_json".to_owned()
        }
    );
    drop(s);
    let mut s2 = connect(&dir);
    send(&mut s2, &Request::Hello);
    assert!(matches!(recv(&mut s2), Response::Hello { .. }));
    drop(s2);
    shutdown(&dir, handle);
}

#[test]
fn idle_ttl_ends_serve() {
    let dir = TempDir::new();
    let started = Instant::now();
    let handle = start(&dir, Fake::default(), Duration::from_millis(300));
    let (res, ended) = handle.join().unwrap();
    assert!(res.is_ok());
    assert!(ended - started >= Duration::from_millis(300));
}

#[test]
fn idle_clock_does_not_fire_during_a_request() {
    let dir = TempDir::new();
    let fake = Fake {
        delay: Duration::from_millis(500),
        ..Fake::default()
    };
    let handle = start(&dir, fake, Duration::from_millis(300));
    let mut s = connect(&dir);
    send(&mut s, &transcribe_req(&[1]));
    assert!(matches!(recv(&mut s), Response::Transcript(_)));
    let responded = Instant::now();
    drop(s);
    let (res, ended) = handle.join().unwrap();
    assert!(res.is_ok());
    // Small slack: the server stamps the response slightly before or after the client wakes.
    assert!(ended - responded >= Duration::from_millis(250));
}

#[test]
fn shutdown_replies_bye_and_returns() {
    let dir = TempDir::new();
    let handle = start(&dir, Fake::default(), LONG);
    shutdown(&dir, handle);
}

#[cfg(unix)]
#[test]
fn bind_replaces_a_stale_socket() {
    let dir = TempDir::new();
    let stale = UnixListener::bind(dir.sock()).unwrap();
    drop(stale);
    assert!(dir.sock().exists());
    bind(&dir.sock()).unwrap();
}

#[test]
fn bind_detects_a_live_listener() {
    let dir = TempDir::new();
    let _live = bind(&dir.sock()).unwrap();
    assert!(matches!(bind(&dir.sock()), Err(BindError::AlreadyRunning)));
}

#[cfg(unix)]
#[test]
fn bind_refuses_to_replace_a_regular_file() {
    let dir = TempDir::new();
    std::fs::write(dir.sock(), b"x").unwrap();
    assert!(matches!(bind(&dir.sock()), Err(BindError::Io(_))));
    assert!(dir.sock().exists());
}

#[cfg(unix)]
#[test]
fn bind_sets_private_modes() {
    let dir = TempDir::new();
    let sub = dir.0.join("hece");
    let path = sub.join("engine.sock");
    let _l = bind(&path).unwrap();
    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&path), 0o600);
    assert_eq!(mode(&sub), 0o700);
}

#[cfg(windows)]
#[test]
fn bind_creates_the_directory_and_lock_file() {
    let dir = TempDir::new();
    let path = dir.0.join("hece").join("engine.sock");
    let _l = bind(&path).unwrap();
    assert!(path.with_file_name("engine.lock").is_file());
}

/// A process that already owns the pipe name, without the lock, is never shared with.
#[cfg(windows)]
#[test]
fn bind_fails_when_another_process_owns_the_pipe_name() {
    use interprocess::local_socket::{GenericFilePath, ListenerOptions, ToFsName};
    let dir = TempDir::new();
    let name = hece_core::ipc::pipe_name(&dir.sock())
        .to_fs_name::<GenericFilePath>()
        .unwrap();
    let _squatter = ListenerOptions::new().name(name).create_sync().unwrap();
    assert!(matches!(bind(&dir.sock()), Err(BindError::Io(_))));
}

#[test]
fn serve_args_parse() {
    let args =
        |v: &[&str]| ServeArgs::parse(&v.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>());
    let a = args(&["--socket", "/tmp/x.sock", "--idle-ttl-secs", "5"]).unwrap();
    assert_eq!(a.socket, Some(PathBuf::from("/tmp/x.sock")));
    assert_eq!(a.idle_ttl, Duration::from_secs(5));
    let d = args(&[]).unwrap();
    assert_eq!(d.socket, None);
    assert!(args(&["--idle-ttl-secs", "x"]).is_err());
    assert!(args(&["--socket"]).is_err());
    assert!(args(&["--bogus"]).is_err());
    assert!(args(&["--idle-ttl-secs", "0"]).is_err());
}

#[test]
fn expired_compares_against_now() {
    let now = Instant::now();
    assert!(!expired(None, now));
    assert!(!expired(Some(now + Duration::from_secs(1)), now));
    assert!(expired(Some(now), now));
}

#[test]
fn watchdog_fires_when_a_request_outlives_its_deadline() {
    let dir = TempDir::new();
    let Bound { listener, lock } = bind(&dir.sock()).unwrap();
    let fired = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let hook = Arc::clone(&fired);
    let handle = thread::spawn(move || {
        let _lock = lock;
        let mut fake = Fake {
            delay: Duration::from_secs(1),
            ..Fake::default()
        };
        let opts = ServeOptions {
            idle_ttl: LONG,
            watchdog_base: Duration::from_millis(200),
            on_expire: Box::new(move || hook.store(true, Ordering::SeqCst)),
        };
        let res = serve_with(&listener, &mut fake, &opts);
        (res, Instant::now())
    });
    let mut s = connect(&dir);
    send(&mut s, &transcribe_req(&[1]));
    thread::sleep(Duration::from_millis(600));
    assert!(fired.load(Ordering::SeqCst), "watchdog did not fire");
    assert!(matches!(recv(&mut s), Response::Transcript(_)));
    drop(s);
    shutdown(&dir, handle);
}

#[test]
fn watchdog_stays_quiet_for_fast_requests() {
    let dir = TempDir::new();
    let Bound { listener, lock } = bind(&dir.sock()).unwrap();
    let fired = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let hook = Arc::clone(&fired);
    let handle = thread::spawn(move || {
        let _lock = lock;
        let opts = ServeOptions {
            idle_ttl: LONG,
            watchdog_base: Duration::from_millis(200),
            on_expire: Box::new(move || hook.store(true, Ordering::SeqCst)),
        };
        let res = serve_with(&listener, &mut Fake::default(), &opts);
        (res, Instant::now())
    });
    let mut s = connect(&dir);
    send(&mut s, &transcribe_req(&[1]));
    assert!(matches!(recv(&mut s), Response::Transcript(_)));
    thread::sleep(Duration::from_millis(500));
    assert!(!fired.load(Ordering::SeqCst));
    drop(s);
    shutdown(&dir, handle);
}
