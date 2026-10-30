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
    DropAfterAnswer,
}

struct FakeSpawner {
    socket: PathBuf,
    modes: Vec<Mode>,
    spawns: AtomicUsize,
    seen: Arc<Mutex<Vec<Request>>>,
    overlap: Arc<AtomicBool>,
    pids: Mutex<Vec<u32>>,
}

fn pid_alive(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

async fn wait_gone(pid: u32) {
    for _ in 0..100 {
        if !pid_alive(pid) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("process {pid} still alive");
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
        let server = tokio::spawn(async move {
            serve(listener, mode, seen, overlap).await;
            let _ = std::fs::remove_file(&socket);
        });
        let child = Command::new("sleep")
            .arg("60")
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| EngineClientError::Spawn(e.to_string()))?;
        if let Some(pid) = child.id() {
            self.pids.lock().unwrap().push(pid);
            // The fake worker dies with its process, like a real one.
            tokio::spawn(async move {
                wait_gone(pid).await;
                server.abort();
            });
        }
        Ok(child)
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
                    Mode::DropAfterAnswer => {
                        write_resp(&mut s, &transcript("hello")).await;
                        break;
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
        pids: Mutex::default(),
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

#[tokio::test]
async fn stale_connection_is_replaced_transparently() {
    let f = fixture("stale", vec![Mode::DropAfterAnswer], Duration::from_secs(5));
    assert_eq!(tx(&f.client, None).await.unwrap().text, "hello");
    assert_eq!(tx(&f.client, None).await.unwrap().text, "hello");
    assert_eq!(f.spawner.spawns.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn recovers_after_timeout() {
    let f = fixture(
        "recover",
        vec![Mode::Silent, Mode::Normal],
        Duration::from_millis(200),
    );
    assert!(matches!(
        tx(&f.client, None).await,
        Err(EngineClientError::Timeout)
    ));
    let pid = f.spawner.pids.lock().unwrap()[0];
    wait_gone(pid).await;
    // Let the fake worker notice its process is gone.
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(tx(&f.client, None).await.unwrap().text, "hello");
    assert_eq!(f.spawner.spawns.load(Ordering::SeqCst), 2);
}

struct NeverBinds {
    pid: Mutex<Option<u32>>,
}

impl Spawner for NeverBinds {
    fn spawn(&self, _cfg: &EngineConfig) -> Result<Child, EngineClientError> {
        let child = Command::new("sleep")
            .arg("30")
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| EngineClientError::Spawn(e.to_string()))?;
        *self.pid.lock().unwrap() = child.id();
        Ok(child)
    }
}

#[tokio::test]
async fn connect_timeout_kills_the_unbound_worker() {
    let mut c = cfg(Path::new("/tmp/hec-never.sock"), Duration::from_secs(1));
    c.connect_timeout = Duration::from_millis(300);
    let spawner = Arc::new(NeverBinds {
        pid: Mutex::new(None),
    });
    let client = EngineClient::new(c, spawner.clone());
    let err = client.prewarm().await.unwrap_err();
    assert!(matches!(err, EngineClientError::ConnectTimeout), "{err}");
    let pid = spawner.pid.lock().unwrap().unwrap();
    assert!(!pid_alive(pid));
}

struct OtherOwnsSocket {
    socket: PathBuf,
    spawns: AtomicUsize,
}

impl Spawner for OtherOwnsSocket {
    fn spawn(&self, _cfg: &EngineConfig) -> Result<Child, EngineClientError> {
        self.spawns.fetch_add(1, Ordering::SeqCst);
        let socket = self.socket.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            let listener = UnixListener::bind(&socket).unwrap();
            serve(listener, Mode::Normal, Arc::default(), Arc::default()).await;
        });
        Command::new("sh")
            .args(["-c", "exit 3"])
            .spawn()
            .map_err(|e| EngineClientError::Spawn(e.to_string()))
    }
}

#[tokio::test]
async fn exit_code_3_keeps_retrying_until_the_other_worker_answers() {
    let f = fixture("exit3", vec![Mode::Normal], Duration::from_secs(5));
    let spawner = Arc::new(OtherOwnsSocket {
        socket: f.dir.join("o.sock"),
        spawns: AtomicUsize::new(0),
    });
    let client = EngineClient::new(
        cfg(&f.dir.join("o.sock"), Duration::from_secs(5)),
        spawner.clone(),
    );
    assert_eq!(tx(&client, None).await.unwrap().text, "hello");
    assert_eq!(spawner.spawns.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn shutdown_without_worker_is_ok_and_with_worker_sends_shutdown() {
    let f = fixture("shutdown", vec![Mode::Normal], Duration::from_secs(5));
    f.client.shutdown().await.unwrap();
    assert_eq!(f.spawner.spawns.load(Ordering::SeqCst), 0);
    tx(&f.client, None).await.unwrap();
    f.client.shutdown().await.unwrap();
    assert!(
        f.spawner
            .seen
            .lock()
            .unwrap()
            .iter()
            .any(|r| matches!(r, Request::Shutdown))
    );
}

#[test]
fn default_config_needs_a_runtime_dir() {
    let m = PathBuf::from("/models/m");
    for dir in [None, Some(std::ffi::OsString::new())] {
        let err = EngineConfig::with_runtime_dir(m.clone(), dir).unwrap_err();
        assert!(matches!(err, EngineClientError::NoRuntimeDir));
        assert_eq!(err.code(), "no_runtime_dir");
    }
    let c = EngineConfig::with_runtime_dir(m, Some("/run/user/1".into())).unwrap();
    assert_eq!(c.socket, Path::new("/run/user/1/hece/engine.sock"));
    assert_eq!(c.compute, "int8_float16");
}
