//! Local OpenAI-compatible API so other apps can share Hece's providers, vocabulary, quality
//! layers and loaded model. Off by default; binds 127.0.0.1 only; every transcription needs the
//! bearer token from a per-user file. Hece-specific request fields start with `hece_` and the
//! response adds a `hece` object, both ignored by standard clients.

use std::io::Cursor;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::{DefaultBodyLimit, Multipart, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use hece_core::ipc::MAX_AUDIO_SAMPLES;
use hece_core::{Transcript, Vocabulary};
use hece_providers::{Cleaner, Providers};
use serde_json::json;

/// A client gets this long to send its request headers.
const HEADER_TIMEOUT: Duration = Duration::from_secs(10);
/// ... and this long to send the whole upload (25 MB over loopback takes well under a second).
const BODY_TIMEOUT: Duration = Duration::from_secs(60);

/// Largest accepted upload (`OpenAI`'s limit).
pub const MAX_UPLOAD_BYTES: usize = 25 * 1024 * 1024;

/// What the endpoints share with the daemon.
pub struct ApiState {
    pub providers: Arc<Providers>,
    pub cleaner: Option<Arc<Cleaner>>,
    pub vocabulary: Option<Arc<Vocabulary>>,
    pub language: Option<String>,
    pub prompt_tokens: usize,
    pub token: String,
    /// Transcriptions running at once; more get 503 instead of queueing audio in memory.
    pub slots: tokio::sync::Semaphore,
}

/// Concurrent transcriptions the API accepts.
pub const MAX_CONCURRENT: usize = 2;

/// An `OpenAI`-style error body with a stable code.
fn error(status: StatusCode, code: &str, message: &str) -> Response {
    let body = json!({"error": {"message": message, "type": "hece_error", "code": code}});
    (status, axum::Json(body)).into_response()
}

/// Constant-time comparison so the token cannot be guessed byte by byte from timings.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn authorized(headers: &HeaderMap, token: &str) -> bool {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(|t| same(t.trim().as_bytes(), token.as_bytes()))
}

/// Rejects requests whose `Host` is not this machine (DNS rebinding from a web page).
fn local_host(headers: &HeaderMap) -> bool {
    headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(|h| h.rsplit_once(':').map_or(h, |(host, _)| host))
        .is_some_and(|h| matches!(h, "127.0.0.1" | "localhost" | "[::1]"))
}

/// Why an upload is not decoded.
enum Rejected {
    Unsupported,
    TooLong,
}

/// Decodes any PCM/float WAV to 16 kHz mono. The header's rate and length size the decode and
/// the resampler, so odd rates and over-long files are refused before anything is allocated.
fn decode_wav(bytes: &[u8]) -> Result<Vec<i16>, Rejected> {
    let reader = hound::WavReader::new(Cursor::new(bytes)).map_err(|_| Rejected::Unsupported)?;
    let spec = reader.spec();
    if !(8_000..=192_000).contains(&spec.sample_rate) {
        return Err(Rejected::Unsupported);
    }
    if u64::from(reader.duration()) * 16_000 / u64::from(spec.sample_rate)
        > MAX_AUDIO_SAMPLES as u64
    {
        return Err(Rejected::TooLong);
    }
    let samples: Vec<f32> = match (spec.sample_format, spec.bits_per_sample) {
        (hound::SampleFormat::Float, 32) => reader
            .into_samples::<f32>()
            .collect::<Result<_, _>>()
            .map_err(|_| Rejected::Unsupported)?,
        (hound::SampleFormat::Int, bits @ 8..=32) => {
            let scale = 1.0 / f64::from(1u32 << (bits - 1));
            reader
                .into_samples::<i32>()
                .map(|s| {
                    #[expect(
                        clippy::cast_possible_truncation,
                        reason = "normalized to [-1, 1], well within f32"
                    )]
                    s.map(|v| (f64::from(v) * scale) as f32)
                })
                .collect::<Result<_, _>>()
                .map_err(|_| Rejected::Unsupported)?
        }
        _ => return Err(Rejected::Unsupported),
    };
    let mono = hece_audio::resample::downmix(&samples, spec.channels);
    hece_audio::resample::to_16k(&mono, spec.sample_rate).map_err(|_| Rejected::Unsupported)
}

#[derive(Default)]
struct Form {
    file: Option<Vec<u8>>,
    language: Option<String>,
    prompt: Option<String>,
    response_format: Option<String>,
    vocabulary: Option<String>,
    cleanup: bool,
}

/// Reads the multipart form within [`BODY_TIMEOUT`].
async fn read_form(multipart: Multipart) -> Result<Form, Box<Response>> {
    tokio::time::timeout(BODY_TIMEOUT, read_fields(multipart))
        .await
        .unwrap_or_else(|_| {
            Err(Box::new(error(
                StatusCode::REQUEST_TIMEOUT,
                "upload_timeout",
                "the upload took too long",
            )))
        })
}

async fn read_fields(mut multipart: Multipart) -> Result<Form, Box<Response>> {
    let mut form = Form::default();
    loop {
        let field = match multipart.next_field().await {
            Ok(Some(field)) => field,
            Ok(None) => break,
            Err(e) => {
                let status = e.status();
                let code = if status == StatusCode::PAYLOAD_TOO_LARGE {
                    "upload_too_large"
                } else {
                    "bad_multipart"
                };
                return Err(Box::new(error(status, code, &e.body_text())));
            }
        };
        let name = field.name().unwrap_or_default().to_owned();
        if name == "file" {
            let bytes = field.bytes().await.map_err(|e| {
                Box::new(error(
                    e.status(),
                    "upload_too_large",
                    "the upload is too large or broken",
                ))
            })?;
            form.file = Some(bytes.to_vec());
            continue;
        }
        let Ok(value) = field.text().await else {
            return Err(Box::new(error(
                StatusCode::BAD_REQUEST,
                "bad_multipart",
                "unreadable field",
            )));
        };
        let value = Some(value.trim().to_owned()).filter(|v| !v.is_empty());
        match name.as_str() {
            "language" => form.language = value,
            "prompt" => form.prompt = value,
            "response_format" => form.response_format = value,
            "hece_vocabulary" => form.vocabulary = value,
            "hece_cleanup" => form.cleanup = value.as_deref() == Some("true"),
            // `model`, `temperature` and other OpenAI fields are accepted and ignored.
            _ => {}
        }
    }
    Ok(form)
}

async fn health(headers: HeaderMap) -> Response {
    if !local_host(&headers) {
        return error(
            StatusCode::FORBIDDEN,
            "bad_host",
            "only local requests are served",
        );
    }
    axum::Json(json!({"status": "ok"})).into_response()
}

async fn transcriptions(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    multipart: Multipart,
) -> Response {
    if !local_host(&headers) {
        return error(
            StatusCode::FORBIDDEN,
            "bad_host",
            "only local requests are served",
        );
    }
    if !authorized(&headers, &state.token) {
        return error(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "missing or wrong bearer token",
        );
    }
    let Ok(_slot) = state.slots.try_acquire() else {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "busy",
            "too many transcriptions at once; retry shortly",
        );
    };
    let form = match read_form(multipart).await {
        Ok(form) => form,
        Err(response) => return *response,
    };
    let Some(file) = form.file else {
        return error(
            StatusCode::BAD_REQUEST,
            "missing_file",
            "the file field is required",
        );
    };
    let format = form.response_format.as_deref().unwrap_or("json");
    if !matches!(format, "json" | "text" | "verbose_json") {
        return error(
            StatusCode::BAD_REQUEST,
            "bad_format",
            "response_format must be json, text or verbose_json",
        );
    }
    let decoded = tokio::task::spawn_blocking(move || decode_wav(&file))
        .await
        .unwrap_or(Err(Rejected::Unsupported));
    let audio = match decoded {
        Ok(audio) if audio.len() <= MAX_AUDIO_SAMPLES => audio,
        Ok(_) | Err(Rejected::TooLong) => {
            return error(
                StatusCode::PAYLOAD_TOO_LARGE,
                "audio_too_long",
                "audio is longer than 11 minutes",
            );
        }
        Err(Rejected::Unsupported) => {
            return error(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "unsupported_audio",
                "only WAV audio is supported",
            );
        }
    };
    // Per-request vocabulary: `hece_vocabulary` (one term per line), else the `prompt` terms.
    let request_vocab = form.vocabulary.map(|v| Vocabulary::parse(&v)).or_else(|| {
        form.prompt
            .map(|p| Vocabulary::parse(&p.replace(',', "\n")))
    });
    let vocabulary = request_vocab
        .map(Arc::new)
        .or_else(|| state.vocabulary.clone());
    let language = form.language.or_else(|| state.language.clone());
    let params = crate::longform::Params {
        language,
        vocabulary,
        prompt_tokens: state.prompt_tokens,
    };
    let transcript = match crate::longform::transcribe(&state.providers, &audio, &params).await {
        Ok(outcome) => outcome.transcript.with_quality_layers(),
        Err(e) => {
            tracing::warn!(code = e.code(), "api transcription failed");
            return error(StatusCode::BAD_GATEWAY, e.code(), &e.to_string());
        }
    };
    let (text, cleanup_skipped) = match (&state.cleaner, form.cleanup) {
        (Some(cleaner), true) if !transcript.text.is_empty() => {
            let c = cleaner.guarded(&transcript.text).await;
            (c.text, c.skipped)
        }
        _ => (transcript.text.clone(), None),
    };
    respond(
        format,
        &transcript,
        text,
        cleanup_skipped.as_deref(),
        audio.len(),
    )
}

fn respond(
    format: &str,
    transcript: &Transcript,
    text: String,
    cleanup_skipped: Option<&str>,
    samples: usize,
) -> Response {
    if format == "text" {
        return text.into_response();
    }
    let hece = json!({
        "provider": transcript.provider,
        "low_confidence": transcript.low_confidence,
        "cleanup_skipped": cleanup_skipped,
    });
    if format == "json" {
        return axum::Json(json!({"text": text, "hece": hece})).into_response();
    }
    #[expect(
        clippy::cast_precision_loss,
        reason = "sample counts are far below 2^52"
    )]
    let duration = samples as f64 / 16_000.0;
    let segments: Vec<_> = transcript
        .segments
        .iter()
        .flatten()
        .enumerate()
        .map(|(id, s)| {
            json!({"id": id, "start": s.start_s, "end": s.end_s, "text": s.text,
                   "avg_logprob": s.avg_logprob, "no_speech_prob": s.no_speech_prob})
        })
        .collect();
    axum::Json(json!({
        "task": "transcribe",
        "language": transcript.language,
        "duration": duration,
        "text": text,
        "segments": segments,
        "hece": hece,
    }))
    .into_response()
}

pub fn router(state: Arc<ApiState>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v1/audio/transcriptions", post(transcriptions))
        // Room for the multipart framing around a maximal file.
        .layer(DefaultBodyLimit::max(MAX_UPLOAD_BYTES + 64 * 1024))
        .with_state(state)
}

/// Reads the token from `path`, creating a random one (owner-only file) when missing.
pub fn token(path: &Path) -> anyhow::Result<String> {
    if let Ok(existing) = std::fs::read_to_string(path) {
        let existing = existing.trim().to_owned();
        if existing.len() >= 32 {
            return Ok(existing);
        }
    }
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| anyhow::anyhow!("no randomness: {e}"))?;
    let token = bytes.iter().fold(String::with_capacity(64), |mut s, b| {
        let _ = std::fmt::Write::write_fmt(&mut s, format_args!("{b:02x}"));
        s
    });
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    std::io::Write::write_all(&mut opts.open(path)?, token.as_bytes())?;
    Ok(token)
}

/// Serves the API on 127.0.0.1:`port` until the process ends.
pub async fn serve(state: Arc<ApiState>, port: u16) -> anyhow::Result<SocketAddr> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    let addr = listener.local_addr()?;
    let app = router(state);
    tokio::spawn(async move {
        loop {
            let stream = match listener.accept().await {
                Ok((stream, _)) => stream,
                Err(e) => {
                    tracing::warn!(kind = ?e.kind(), "local API accept failed");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
            };
            let service = hyper_util::service::TowerToHyperService::new(app.clone());
            tokio::spawn(async move {
                // A client that never finishes its headers is dropped instead of holding a task.
                let conn = hyper::server::conn::http1::Builder::new()
                    .timer(hyper_util::rt::TokioTimer::new())
                    .header_read_timeout(HEADER_TIMEOUT)
                    .serve_connection(hyper_util::rt::TokioIo::new(stream), service);
                if let Err(e) = conn.await {
                    tracing::debug!(error = %e, "local API connection ended");
                }
            });
        }
    });
    Ok(addr)
}

#[cfg(test)]
mod tests {
    use hece_providers::{
        BoxFuture, Capabilities, Provider, ProviderError, ProviderRequest, VocabularyStyle,
    };
    use reqwest::multipart::{Form as Multi, Part};

    use super::*;

    #[derive(Debug)]
    struct Echo(Capabilities);

    impl Provider for Echo {
        fn name(&self) -> &'static str {
            "echo"
        }
        fn capabilities(&self) -> &Capabilities {
            &self.0
        }
        fn transcribe<'a>(
            &'a self,
            req: ProviderRequest<'a>,
        ) -> BoxFuture<'a, Result<Transcript, ProviderError>> {
            Box::pin(async move {
                Ok(Transcript {
                    text: format!("{} samples", req.audio.len()),
                    segments: None,
                    language: req.language.unwrap_or_default().to_owned(),
                    provider: "echo".to_owned(),
                    low_confidence: false,
                })
            })
        }
    }

    async fn start() -> String {
        let caps = Capabilities {
            local: true,
            vocabulary: VocabularyStyle::Unsupported,
            segments: false,
            max_audio_samples: MAX_AUDIO_SAMPLES,
            language_detection: true,
        };
        let providers = Providers::new(Arc::new(Echo(caps)), None, false).unwrap();
        let state = Arc::new(ApiState {
            providers: Arc::new(providers),
            cleaner: None,
            vocabulary: None,
            language: None,
            prompt_tokens: 200,
            token: "t".repeat(64),
            slots: tokio::sync::Semaphore::new(MAX_CONCURRENT),
        });
        let addr = serve(state, 0).await.unwrap();
        format!("http://127.0.0.1:{}", addr.port())
    }

    /// One second of 48 kHz stereo silence.
    fn wav_48k_stereo() -> Vec<u8> {
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut out = Cursor::new(Vec::new());
        let mut w = hound::WavWriter::new(&mut out, spec).unwrap();
        for _ in 0..96_000 {
            w.write_sample(0i16).unwrap();
        }
        w.finalize().unwrap();
        out.into_inner()
    }

    async fn post(base: &str, token: Option<&str>, file: Vec<u8>) -> reqwest::Response {
        let form = Multi::new()
            .part("file", Part::bytes(file).file_name("a.wav"))
            .text("model", "whisper-1")
            .text("language", "tr");
        let mut req = reqwest::Client::new()
            .post(format!("{base}/v1/audio/transcriptions"))
            .multipart(form);
        if let Some(token) = token {
            req = req.bearer_auth(token);
        }
        req.send().await.unwrap()
    }

    #[tokio::test]
    async fn openai_style_request_is_resampled_and_answered() {
        let base = start().await;
        let resp = post(&base, Some(&"t".repeat(64)), wav_48k_stereo()).await;
        assert_eq!(resp.status(), 200);
        let body: serde_json::Value = serde_json::from_slice(&resp.bytes().await.unwrap()).unwrap();
        // 48 kHz stereo for 1 s → 16 000 mono samples.
        assert_eq!(body["text"], "16000 samples");
        assert_eq!(body["hece"]["provider"], "echo");
    }

    #[tokio::test]
    async fn missing_or_wrong_token_is_401() {
        let base = start().await;
        // An empty file: the 401 goes out before the body is read, and on Windows a large
        // unread body can make the closing socket reset the connection before the client
        // sees the response.
        assert_eq!(post(&base, None, Vec::new()).await.status(), 401);
        assert_eq!(post(&base, Some("nope"), Vec::new()).await.status(), 401);
    }

    #[tokio::test]
    async fn oversize_upload_is_413_and_non_wav_is_415() {
        let base = start().await;
        let token = "t".repeat(64);
        let big = vec![0u8; MAX_UPLOAD_BYTES + 128 * 1024];
        assert_eq!(post(&base, Some(&token), big).await.status(), 413);
        assert_eq!(
            post(&base, Some(&token), b"ID3 mp3".to_vec())
                .await
                .status(),
            415
        );
        // A header rate of 1 Hz would make the resampler allocate ~16000x the input.
        let mut absurd_rate = wav_48k_stereo();
        absurd_rate[24..28].copy_from_slice(&1u32.to_le_bytes());
        absurd_rate[28..32].copy_from_slice(&4u32.to_le_bytes());
        assert_eq!(post(&base, Some(&token), absurd_rate).await.status(), 415);
    }
}
