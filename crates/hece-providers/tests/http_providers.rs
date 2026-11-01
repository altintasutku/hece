//! Contract tests for the HTTP adapters against a mock server: request shape, response
//! parsing and error mapping (401, 429, timeout, 5xx, bad body).
#![allow(clippy::unwrap_used, reason = "tests may unwrap")]

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use hece_core::Vocabulary;
use hece_core::ipc::MAX_AUDIO_SAMPLES;
use hece_providers::{
    ApiKey, DeepgramConfig, DeepgramProvider, KeySource, OpenAiConfig, OpenAiProvider, Provider,
    ProviderError, ProviderRequest, SecretStore, StoreError,
};
use serde_json::json;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Match, Mock, MockServer, Request, ResponseTemplate};

#[derive(Debug, Default)]
struct MemoryStore {
    keys: Mutex<HashMap<String, String>>,
    reads: AtomicUsize,
}

impl SecretStore for MemoryStore {
    fn get(&self, account: &str) -> Result<Option<ApiKey>, StoreError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        Ok(self
            .keys
            .lock()
            .unwrap()
            .get(account)
            .map(|k| ApiKey::new(k)))
    }

    fn set(&self, account: &str, key: &ApiKey) -> Result<(), StoreError> {
        self.keys
            .lock()
            .unwrap()
            .insert(account.to_owned(), key.expose().to_owned());
        Ok(())
    }

    fn delete(&self, account: &str) -> Result<(), StoreError> {
        self.keys.lock().unwrap().remove(account);
        Ok(())
    }
}

fn key_for(name: &str, key: &str) -> (Arc<KeySource>, Arc<MemoryStore>) {
    let store = Arc::new(MemoryStore::default());
    store.set(name, &ApiKey::new(key)).unwrap();
    let src = KeySource::new(name, None, store.clone()).with_env(Arc::new(|_| None));
    (Arc::new(src), store)
}

const AUDIO: [i16; 1600] = [0; 1600];

fn request<'a>(language: Option<&'a str>, vocab: Option<&'a Vocabulary>) -> ProviderRequest<'a> {
    ProviderRequest {
        audio: &AUDIO,
        language,
        vocabulary: vocab,
        prompt_prefix: "Vocabulary: ",
        prompt_tokens: 200,
    }
}

fn vocab(terms: &[&str]) -> Vocabulary {
    Vocabulary {
        terms: terms.iter().map(|s| (*s).to_owned()).collect(),
    }
}

fn openai(server: &MockServer, key: Option<Arc<KeySource>>, segments: bool) -> OpenAiProvider {
    let mut cfg = OpenAiConfig::new("openai", &format!("{}/v1", server.uri()), "whisper-1");
    cfg.segments = segments;
    cfg.timeout = Duration::from_millis(500);
    OpenAiProvider::new(cfg, key).unwrap()
}

fn deepgram(server: &MockServer, key: Arc<KeySource>) -> DeepgramProvider {
    let mut cfg = DeepgramConfig::new("deepgram");
    cfg.base_url = server.uri();
    cfg.timeout = Duration::from_millis(500);
    DeepgramProvider::new(cfg, key).unwrap()
}

fn form_field(name: &str, value: &str) -> String {
    format!("name=\"{name}\"\r\n\r\n{value}\r\n")
}

/// Matches a byte sequence anywhere in the body; multipart bodies with WAV data are not UTF-8,
/// so wiremock's string matcher never matches them.
struct BodyHas(Vec<u8>);

impl Match for BodyHas {
    fn matches(&self, request: &Request) -> bool {
        request.body.windows(self.0.len()).any(|w| w == self.0)
    }
}

fn body_has(needle: impl AsRef<[u8]>) -> BodyHas {
    BodyHas(needle.as_ref().to_vec())
}

// ---------- OpenAI-compatible ----------

#[tokio::test]
async fn openai_sends_multipart_with_key_prompt_and_language() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/audio/transcriptions"))
        .and(header("authorization", "Bearer sk-test"))
        .and(body_has(form_field("model", "whisper-1")))
        .and(body_has(form_field("response_format", "json")))
        .and(body_has(form_field("language", "tr")))
        .and(body_has(form_field(
            "prompt",
            "Vocabulary: Kubernetes, Tauri.",
        )))
        .and(body_has("filename=\"audio.wav\""))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"text": " merhaba dünya "})))
        .expect(1)
        .mount(&server)
        .await;
    let (key, _) = key_for("openai", "sk-test");
    let p = openai(&server, Some(key), false);
    let v = vocab(&["Kubernetes", "Tauri"]);
    let t = p.transcribe(request(Some("tr"), Some(&v))).await.unwrap();
    assert_eq!(t.text, "merhaba dünya");
    assert_eq!(t.language, "tr");
    assert_eq!(t.provider, "openai");
    assert!(t.segments.is_none());
}

#[tokio::test]
async fn openai_verbose_json_yields_segments() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_has(form_field("response_format", "verbose_json")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "text": "hello there",
            "language": "english",
            "segments": [
                {"id": 0, "start": 0.0, "end": 1.5, "text": " hello there",
                 "avg_logprob": -0.25, "no_speech_prob": 0.01}
            ]
        })))
        .mount(&server)
        .await;
    let p = openai(&server, None, true);
    assert!(p.capabilities().segments);
    let t = p.transcribe(request(None, None)).await.unwrap();
    let segs = t.segments.unwrap();
    assert_eq!(segs.len(), 1);
    assert!((segs[0].avg_logprob + 0.25).abs() < f32::EPSILON);
    // A language name is not a code, so it is dropped.
    assert_eq!(t.language, "");
}

#[tokio::test]
async fn openai_without_key_sends_no_authorization() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"text": "ok", "language": "en"})),
        )
        .mount(&server)
        .await;
    let p = openai(&server, None, false);
    assert!(p.capabilities().local, "a loopback server is local");
    let t = p.transcribe(request(None, None)).await.unwrap();
    assert_eq!(t.language, "en");
    let received = server.received_requests().await.unwrap();
    assert!(received[0].headers.get("authorization").is_none());
}

#[tokio::test]
async fn openai_401_is_unauthorized_and_rereads_the_key() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(401).set_body_string("{\"error\":\"bad key\"}"))
        .mount(&server)
        .await;
    let (key, store) = key_for("openai", "sk-old");
    let p = openai(&server, Some(key), false);
    let e = p.transcribe(request(None, None)).await.unwrap_err();
    assert_eq!(e.code(), "provider_unauthorized");
    p.transcribe(request(None, None)).await.unwrap_err();
    assert_eq!(store.reads.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn openai_429_carries_retry_after() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "7"))
        .mount(&server)
        .await;
    let p = openai(&server, None, false);
    match p.transcribe(request(None, None)).await.unwrap_err() {
        ProviderError::RateLimited { retry_after, .. } => {
            assert_eq!(retry_after, Some(Duration::from_secs(7)));
        }
        other => panic!("expected rate limit, got {other:?}"),
    }
}

#[tokio::test]
async fn openai_slow_server_times_out() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"text": "late"}))
                .set_delay(Duration::from_secs(3)),
        )
        .mount(&server)
        .await;
    let p = openai(&server, None, false);
    let e = p.transcribe(request(None, None)).await.unwrap_err();
    assert_eq!(e.code(), "provider_timeout");
}

#[tokio::test]
async fn openai_server_error_and_bad_body_are_typed() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
        .mount(&server)
        .await;
    let p = openai(&server, None, false);
    let e = p.transcribe(request(None, None)).await.unwrap_err();
    assert_eq!(e.code(), "provider_server_error");
    let e = p.transcribe(request(None, None)).await.unwrap_err();
    assert_eq!(e.code(), "provider_bad_response");
}

#[tokio::test]
async fn openai_rejects_overlong_audio_without_a_request() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    let p = openai(&server, None, false);
    let long = vec![0i16; MAX_AUDIO_SAMPLES + 1];
    let req = ProviderRequest {
        audio: &long,
        ..request(None, None)
    };
    let e = p.transcribe(req).await.unwrap_err();
    assert_eq!(e.code(), "audio_too_long");
}

#[test]
fn openai_refuses_to_send_a_key_over_plain_http() {
    let (key, _) = key_for("openai", "sk");
    let cfg = OpenAiConfig::new("openai", "http://example.com/v1", "whisper-1");
    let e = OpenAiProvider::new(cfg, Some(key)).unwrap_err();
    assert_eq!(e.code(), "provider_config");
}

// ---------- Deepgram ----------

#[tokio::test]
async fn deepgram_sends_token_wav_and_one_keyterm_per_term() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/listen"))
        .and(header("authorization", "Token dg-test"))
        .and(header("content-type", "audio/wav"))
        .and(query_param("model", "nova-3"))
        .and(query_param("language", "tr"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "metadata": {},
            "results": {"channels": [{"alternatives": [{"transcript": "merhaba", "confidence": 0.9}]}]}
        })))
        .expect(1)
        .mount(&server)
        .await;
    let (key, _) = key_for("deepgram", "dg-test");
    let p = deepgram(&server, key);
    let v = vocab(&["Kubernetes", "customer service"]);
    let t = p.transcribe(request(Some("tr"), Some(&v))).await.unwrap();
    assert_eq!(t.text, "merhaba");
    assert_eq!(t.language, "tr");

    let received = server.received_requests().await.unwrap();
    let keyterms: Vec<String> = received[0]
        .url
        .query_pairs()
        .filter(|(k, _)| k == "keyterm")
        .map(|(_, v)| v.into_owned())
        .collect();
    assert_eq!(keyterms, ["Kubernetes", "customer service"]);
    assert_eq!(&received[0].body[..4], b"RIFF");
}

#[tokio::test]
async fn deepgram_detects_language_when_none_is_set() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(query_param("detect_language", "true"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": {"channels": [{
                "alternatives": [{"transcript": "hello"}],
                "detected_language": "en"
            }]}
        })))
        .mount(&server)
        .await;
    let (key, _) = key_for("deepgram", "dg");
    let t = deepgram(&server, key)
        .transcribe(request(None, None))
        .await
        .unwrap();
    assert_eq!(t.language, "en");
    assert!(t.segments.is_none());
}

#[tokio::test]
async fn deepgram_maps_401_429_and_empty_results() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(403))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(429))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"results": {"channels": []}})),
        )
        .mount(&server)
        .await;
    let (key, _) = key_for("deepgram", "dg");
    let p = deepgram(&server, key);
    let mut seen = Vec::new();
    for _ in 0..3 {
        seen.push(
            p.transcribe(request(None, None))
                .await
                .unwrap_err()
                .code()
                .to_owned(),
        );
    }
    assert_eq!(
        seen,
        [
            "provider_unauthorized",
            "provider_rate_limited",
            "provider_bad_response"
        ]
    );
}

#[tokio::test]
async fn deepgram_missing_key_fails_before_sending() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    let src = KeySource::new("deepgram", None, Arc::new(MemoryStore::default()))
        .with_env(Arc::new(|_| None));
    let e = deepgram(&server, Arc::new(src))
        .transcribe(request(None, None))
        .await
        .unwrap_err();
    assert_eq!(e.code(), "provider_key_missing");
}
