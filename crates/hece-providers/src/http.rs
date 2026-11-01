//! Shared plumbing of the HTTP adapters: endpoint validation, WAV encoding, sending with
//! status mapping, and a size-capped body read. Bodies are never logged (they echo prompts and
//! carry transcripts); only status codes are.

use std::io::Cursor;
use std::time::Duration;

use reqwest::{RequestBuilder, StatusCode, Url};

use crate::provider::ProviderError;

/// Largest response body accepted.
const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
/// Samples per second of the audio handed to providers.
const SAMPLE_RATE: u32 = 16_000;
/// Time allowed to open a connection.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Builds the HTTP client every adapter instance keeps.
pub(crate) fn client(provider: &str) -> Result<reqwest::Client, ProviderError> {
    reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .build()
        .map_err(|_| config(provider, "http client could not be built"))
}

pub(crate) fn config(provider: &str, reason: &str) -> ProviderError {
    ProviderError::Config {
        provider: provider.to_owned(),
        reason: reason.to_owned(),
    }
}

/// Whether `url` points at this machine.
pub(crate) fn is_loopback(url: &Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    let host = host.trim_start_matches('[').trim_end_matches(']');
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// Parses `base` + `path`. Plain `http` is allowed only for loopback hosts or when no key is
/// sent, so a key never crosses the network in clear text.
pub(crate) fn endpoint(
    provider: &str,
    base: &str,
    path: &str,
    sends_key: bool,
) -> Result<Url, ProviderError> {
    let url = Url::parse(&format!("{}{path}", base.trim_end_matches('/')))
        .map_err(|_| config(provider, "base URL is not a valid URL"))?;
    match url.scheme() {
        "https" => Ok(url),
        "http" if !sends_key || is_loopback(&url) => Ok(url),
        "http" => Err(config(
            provider,
            "an API key is only sent over https (or to localhost)",
        )),
        _ => Err(config(provider, "base URL must be http or https")),
    }
}

/// Encodes 16 kHz mono samples as a 16-bit PCM WAV file.
pub(crate) fn wav(provider: &str, samples: &[i16]) -> Result<Vec<u8>, ProviderError> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut out = Cursor::new(Vec::with_capacity(44 + samples.len() * 2));
    // The only possible failure is a data chunk over 4 GiB.
    write_wav(&mut out, spec, samples).map_err(|_| ProviderError::AudioTooLong {
        provider: provider.to_owned(),
    })?;
    Ok(out.into_inner())
}

fn write_wav(
    out: &mut Cursor<Vec<u8>>,
    spec: hound::WavSpec,
    samples: &[i16],
) -> Result<(), hound::Error> {
    let mut writer = hound::WavWriter::new(out, spec)?;
    for s in samples {
        writer.write_sample(*s)?;
    }
    writer.finalize()
}

/// Request timeout: `base` plus one second per ten seconds of audio.
pub(crate) fn request_timeout(base: Duration, samples: usize) -> Duration {
    let samples = u64::try_from(samples).unwrap_or(u64::MAX);
    base.saturating_add(Duration::from_millis(
        samples.saturating_mul(1000) / u64::from(SAMPLE_RATE) / 10,
    ))
}

fn send_error(provider: &str, e: &reqwest::Error) -> ProviderError {
    let provider = provider.to_owned();
    if e.is_timeout() {
        ProviderError::Timeout { provider }
    } else {
        ProviderError::Network { provider }
    }
}

fn retry_after(resp: &reqwest::Response) -> Option<Duration> {
    resp.headers()
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(Duration::from_secs)
}

fn status_error(provider: &str, status: StatusCode, resp: &reqwest::Response) -> ProviderError {
    let provider = provider.to_owned();
    match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
            ProviderError::Unauthorized { provider }
        }
        StatusCode::TOO_MANY_REQUESTS => ProviderError::RateLimited {
            provider,
            retry_after: retry_after(resp),
        },
        StatusCode::PAYLOAD_TOO_LARGE => ProviderError::AudioTooLong { provider },
        StatusCode::REQUEST_TIMEOUT | StatusCode::GATEWAY_TIMEOUT => {
            ProviderError::Timeout { provider }
        }
        s if s.is_server_error() => ProviderError::Server {
            provider,
            status: s.as_u16(),
        },
        s => ProviderError::Rejected {
            provider,
            status: s.as_u16(),
        },
    }
}

/// Sends `req` and returns the body of a successful response.
pub(crate) async fn send(provider: &str, req: RequestBuilder) -> Result<Vec<u8>, ProviderError> {
    let mut resp = req.send().await.map_err(|e| send_error(provider, &e))?;
    let status = resp.status();
    if !status.is_success() {
        tracing::warn!(
            provider,
            status = status.as_u16(),
            "provider request failed"
        );
        return Err(status_error(provider, status, &resp));
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| send_error(provider, &e))? {
        if body.len() + chunk.len() > MAX_RESPONSE_BYTES {
            tracing::warn!(provider, "provider response too large");
            return Err(bad_response(provider));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

pub(crate) fn bad_response(provider: &str) -> ProviderError {
    ProviderError::BadResponse {
        provider: provider.to_owned(),
    }
}

/// Parses a JSON body; parse failures are [`ProviderError::BadResponse`].
pub(crate) fn parse_json<T: serde::de::DeserializeOwned>(
    provider: &str,
    body: &[u8],
) -> Result<T, ProviderError> {
    serde_json::from_slice(body).map_err(|e| {
        tracing::warn!(
            provider,
            line = e.line(),
            column = e.column(),
            "provider response not understood"
        );
        bad_response(provider)
    })
}

/// Keeps a language value only when it looks like a code (`en`, `tr`, `haw`); some servers
/// return names such as `english`.
pub(crate) fn language_code(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|v| (2..=3).contains(&v.len()) && v.chars().all(|c| c.is_ascii_alphabetic()))
        .map(str::to_ascii_lowercase)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_has_header_and_samples() {
        let bytes = wav("t", &[0, 1, -1]).unwrap();
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(bytes.len(), 44 + 6);
    }

    #[test]
    fn plain_http_only_for_loopback_when_sending_a_key() {
        assert!(endpoint("t", "http://127.0.0.1:8000/v1", "/x", true).is_ok());
        assert!(endpoint("t", "http://localhost:8000/v1/", "/x", true).is_ok());
        assert!(endpoint("t", "http://[::1]:8000", "/x", true).is_ok());
        assert!(endpoint("t", "http://192.168.1.5:8000/v1", "/x", false).is_ok());
        let e = endpoint("t", "http://192.168.1.5:8000/v1", "/x", true).unwrap_err();
        assert_eq!(e.code(), "provider_config");
        assert!(endpoint("t", "ftp://example.com", "/x", false).is_err());
        assert!(endpoint("t", "not a url", "/x", false).is_err());
    }

    #[test]
    fn endpoint_joins_without_double_slash() {
        let url = endpoint(
            "t",
            "https://api.openai.com/v1/",
            "/audio/transcriptions",
            true,
        );
        assert_eq!(
            url.unwrap().as_str(),
            "https://api.openai.com/v1/audio/transcriptions"
        );
    }

    #[test]
    fn timeout_grows_with_audio() {
        let base = Duration::from_secs(60);
        assert_eq!(request_timeout(base, 0), base);
        assert_eq!(request_timeout(base, 16_000 * 100), Duration::from_secs(70));
    }

    #[test]
    fn language_names_are_dropped() {
        assert_eq!(language_code(Some("TR")), Some("tr".into()));
        assert_eq!(language_code(Some("english")), None);
        assert_eq!(language_code(None), None);
    }
}
