//! Tests for the engine worker client against a fake worker on a unix socket.
#![allow(clippy::unwrap_used, reason = "tests may unwrap")]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hece_core::Transcript;
use hece_core::ipc::{PROTOCOL_VERSION, Request, Response, decode_body, encode_frame, frame_len};
use hece_providers::engine_client::{
    EngineClient, EngineClientError, EngineConfig, ProcessSpawner, Spawner,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::process::{Child, Command};

#[derive(Debug, Clone)]
enum Mode {
    Normal,
    CrashOnTranscribe,
    ErrorCode(&'static str),
    Silent,
    BadProtocol,
    Ordered,
}

struct FakeSpawner {
    socket: PathBuf,
    modes: Vec<Mode>,
    spawns: AtomicUsize,
    seen: Arc<Mutex<Vec<Request>>>,
    overlap: Arc<AtomicBool>,
}

impl Spawner for FakeSpawner {
    fn spawn(&self, _cfg: &EngineConfig) -> Result<Child, EngineClientError> {
        let n = self.spawns.fetch_add(1, Ordering::SeqCst);
        let mode = self.modes[n.min(self.modes.len() - 1)].clone();
        let _ = std::fs::remove_file(&self.socket);
        let listener = UnixListener::bind(&self.socket)
            .map_err(|e| EngineClientError::Spawn(e.to_string()))?;
        let socket = self.socket.clone();
        let seen = Arc::clone(&self.seen);
        let overlap = Arc::clone(&self.overlap);
        tokio::spawn(async move {
            serve(listener, mode, seen, overlap).await;
            let _ = std::fs::remove_file(&socket);
        });
        Command::new("sleep")
            .arg("60")
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| EngineClientError::Spawn(e.to_string()))
    }
}

async fn read_req(s: &mut UnixStream) -> Option<Request> {
    let mut h = [0u8; 4];
    s.read_exact(&mut h).await.ok()?;
    let mut body = vec![0u8; frame_len(h).ok()?];
    s.read_exact(&mut body).await.ok()?;
    decode_body(&body).ok()
}

async fn write_resp(s: &mut UnixStream, r: &Response) {
    let _ = s.write_all(&encode_frame(r).unwrap()).await;
}

fn transcript(text: &str) -> Response {
    Response::Transcript(Transcript {
        text: text.to_owned(),
        segments: None,
        language: "en".to_owned(),
        provider: "local".to_owned(),
        low_confidence: false,
    })
}

async fn serve(
    listener: UnixListener,
    mode: Mode,
    seen: Arc<Mutex<Vec<Request>>>,
    overlap: Arc<AtomicBool>,
) {
    while let Ok((mut s, _)) = listener.accept().await {
        while let Some(req) = read_req(&mut s).await {
            seen.lock().unwrap().push(req.clone());
            match req {
                Request::Hello => {
                    let protocol = if matches!(mode, Mode::BadProtocol) {
                        2
                    } else {
                        PROTOCOL_VERSION
                    };
                    write_resp(
                        &mut s,
                        &Response::Hello {
                            protocol,
                            engine_version: "t".to_owned(),
                            loaded_model: None,
                        },
                    )
                    .await;
                }
                Request::Load(_) => write_resp(&mut s, &Response::Loaded { load_ms: 1 }).await,
                Request::Shutdown => write_resp(&mut s, &Response::Bye).await,
                Request::Transcribe(t) => match &mode {
                    Mode::CrashOnTranscribe => return,
                    Mode::ErrorCode(c) => {
                        write_resp(
                            &mut s,
                            &Response::Error {
                                code: (*c).to_owned(),
                            },
                        )
                        .await;
                    }
                    Mode::Silent => tokio::time::sleep(Duration::from_secs(30)).await,
                    Mode::Ordered => {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        let mut b = [0u8; 1];
                        if s.try_read(&mut b).is_ok() {
                            overlap.store(true, Ordering::SeqCst);
                        }
                        write_resp(&mut s, &transcript(t.language.as_deref().unwrap_or(""))).await;
                    }
                    _ => write_resp(&mut s, &transcript("hello")).await,
                },
            }
        }
    }
}

struct Fixture {
    dir: PathBuf,
    client: Arc<EngineClient>,
    spawner: Arc<FakeSpawner>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn cfg(socket: &Path, base: Duration) -> EngineConfig {
    EngineConfig {
        engine_path: None,
        socket: socket.to_path_buf(),
        model_dir: PathBuf::from("/models/m"),
        compute: "float16".to_owned(),
        idle_ttl: Duration::from_secs(60),
        connect_timeout: Duration::from_secs(5),
        base_timeout: base,
    }
}

fn fixture(name: &str, modes: Vec<Mode>, base: Duration) -> Fixture {
    use std::os::unix::fs::DirBuilderExt;
    let dir = std::env::temp_dir().join(format!("hec-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
    let socket = dir.join("e.sock");
    assert!(socket.as_os_str().len() < 108);
    let spawner = Arc::new(FakeSpawner {
        socket: socket.clone(),
        modes,
        spawns: AtomicUsize::new(0),
        seen: Arc::default(),
        overlap: Arc::default(),
    });
    let client = Arc::new(EngineClient::new(cfg(&socket, base), spawner.clone()));
    Fixture {
        dir,
        client,
        spawner,
    }
}

const AUDIO: [i16; 160] = [1; 160];

async fn tx(c: &EngineClient, lang: Option<&str>) -> Result<Transcript, EngineClientError> {
    c.transcribe(&AUDIO, lang, None, "", 0).await
}

#[tokio::test]
async fn first_call_spawns_and_second_reuses() {
    let f = fixture("reuse", vec![Mode::Normal], Duration::from_secs(5));
    assert_eq!(tx(&f.client, Some("en")).await.unwrap().text, "hello");
    assert_eq!(tx(&f.client, Some("en")).await.unwrap().text, "hello");
    assert_eq!(f.spawner.spawns.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn crash_mid_request_then_respawn() {
    let f = fixture(
        "crash",
        vec![Mode::CrashOnTranscribe, Mode::Normal],
        Duration::from_secs(5),
    );
    let err = tx(&f.client, None).await.unwrap_err();
    assert!(matches!(err, EngineClientError::Crashed), "{err}");
    assert_eq!(err.code(), "engine_crashed");
    assert_eq!(tx(&f.client, None).await.unwrap().text, "hello");
    assert_eq!(f.spawner.spawns.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn engine_error_code_passes_through() {
    let f = fixture(
        "err",
        vec![Mode::ErrorCode("cuda_unavailable")],
        Duration::from_secs(5),
    );
    match tx(&f.client, None).await.unwrap_err() {
        EngineClientError::Engine { code } => assert_eq!(code, "cuda_unavailable"),
        other => panic!("unexpected {other}"),
    }
}

#[tokio::test]
async fn silent_worker_times_out() {
    let f = fixture("silent", vec![Mode::Silent], Duration::from_millis(200));
    let start = Instant::now();
    let err = tx(&f.client, None).await.unwrap_err();
    assert!(matches!(err, EngineClientError::Timeout), "{err}");
    assert!(
        start.elapsed() < Duration::from_millis(900),
        "{:?}",
        start.elapsed()
    );
}

#[tokio::test]
async fn concurrent_calls_are_serialized() {
    let f = fixture("order", vec![Mode::Ordered], Duration::from_secs(5));
    let (a, b) = (Arc::clone(&f.client), Arc::clone(&f.client));
    let ta = tokio::spawn(async move { tx(&a, Some("aa")).await });
    let tb = tokio::spawn(async move { tx(&b, Some("bb")).await });
    assert_eq!(ta.await.unwrap().unwrap().text, "aa");
    assert_eq!(tb.await.unwrap().unwrap().text, "bb");
    assert!(
        !f.spawner.overlap.load(Ordering::SeqCst),
        "second frame arrived before first answer"
    );
}

#[tokio::test]
async fn protocol_mismatch_is_reported() {
    let f = fixture("proto", vec![Mode::BadProtocol], Duration::from_secs(5));
    match tx(&f.client, None).await.unwrap_err() {
        EngineClientError::ProtocolMismatch { worker } => assert_eq!(worker, 2),
        other => panic!("unexpected {other}"),
    }
}

#[tokio::test]
async fn prewarm_sends_load_with_config() {
    let f = fixture("prewarm", vec![Mode::Normal], Duration::from_secs(5));
    f.client.prewarm().await.unwrap();
    let seen = f.spawner.seen.lock().unwrap();
    assert!(seen.iter().any(|r| matches!(r, Request::Load(l)
        if l.model_dir == Path::new("/models/m") && l.compute == "float16")));
}

#[tokio::test]
async fn process_spawner_missing_engine() {
    let mut c = cfg(Path::new("/tmp/none.sock"), Duration::from_secs(1));
    c.engine_path = Some(PathBuf::from("/nonexistent/hece-engine"));
    let err = ProcessSpawner.spawn(&c).unwrap_err();
    assert!(matches!(err, EngineClientError::EngineMissing), "{err}");
    assert_eq!(err.code(), "engine_missing");
}
