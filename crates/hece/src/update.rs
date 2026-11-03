//! Update check: the only network call Hece makes on its own. At most once a day it asks
//! GitHub for the latest release (one unauthenticated GET, nothing else sent) and reports a
//! newer stable version. Failures are logged with a code and never shown to the user.

use std::path::Path;
use std::time::Duration;

use serde::Deserialize;

/// GitHub's "latest release" endpoint; it already skips drafts and pre-releases.
pub const LATEST_URL: &str = "https://api.github.com/repos/altintasutku/hece/releases/latest";
/// Release pages, completed with the tag.
const RELEASES_URL: &str = "https://github.com/altintasutku/hece/releases/tag/";
/// Minimum time between two checks.
pub const INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
/// The last check time (Unix seconds) under the state directory.
pub const STATE_FILE: &str = "update-check";
const TIMEOUT: Duration = Duration::from_secs(10);
/// Larger answers are not a release; the body is not read past this.
const MAX_BODY: usize = 256 * 1024;

/// A `major.minor.patch` version. Tags with a pre-release or build suffix do not parse, so
/// they never count as an update.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(u64, u64, u64);

impl Version {
    /// Parses `1.2.3` or `v1.2.3`.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let text = text.strip_prefix('v').unwrap_or(text);
        let mut parts = text.split('.').map(|p| {
            // Digits only: rejects "+1", "-rc1" and empty parts that `parse` would allow.
            (!p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
                .then(|| p.parse::<u64>().ok())
                .flatten()
        });
        let version = Self(parts.next()??, parts.next()??, parts.next()??);
        parts.next().is_none().then_some(version)
    }

    /// The running build.
    #[must_use]
    pub fn current() -> Option<Self> {
        Self::parse(env!("CARGO_PKG_VERSION"))
    }

    /// The release page of this version (built from the parsed numbers, never from the answer).
    #[must_use]
    pub fn page(self) -> String {
        format!("{RELEASES_URL}v{self}")
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

/// Update check failures. [`UpdateError::code`] is the stable code that goes to the log.
#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    /// No answer (offline, DNS, timeout, TLS).
    #[error("GitHub could not be reached")]
    Network,
    /// GitHub answered with an error status (rate limit, no release yet).
    #[error("GitHub answered {0}")]
    Http(u16),
    /// The answer was not a release.
    #[error("the release answer could not be read")]
    Parse,
}

impl UpdateError {
    /// Stable error code.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::Network => "update_network_error",
            Self::Http(_) => "update_http_error",
            Self::Parse => "update_bad_response",
        }
    }
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
}

/// The stable version a release answer names, if any.
fn parse_release(body: &[u8]) -> Result<Option<Version>, UpdateError> {
    let release: Release = serde_json::from_slice(body).map_err(|_| UpdateError::Parse)?;
    if release.draft || release.prerelease {
        return Ok(None);
    }
    Ok(Version::parse(&release.tag_name))
}

/// The client for the check: short timeout, `User-Agent: hece/<version>` (GitHub requires
/// one), no cookies or other headers.
#[must_use]
pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(TIMEOUT)
        .user_agent(concat!("hece/", env!("CARGO_PKG_VERSION")))
        .build()
        .unwrap_or_default()
}

/// Asks `url` for the latest release and returns its version when newer than `current`.
pub async fn latest(
    client: &reqwest::Client,
    url: &str,
    current: Version,
) -> Result<Option<Version>, UpdateError> {
    let mut response = client
        .get(url)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .map_err(|_| UpdateError::Network)?;
    if !response.status().is_success() {
        return Err(UpdateError::Http(response.status().as_u16()));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| UpdateError::Network)? {
        body.extend_from_slice(&chunk);
        if body.len() > MAX_BODY {
            return Err(UpdateError::Parse);
        }
    }
    Ok(parse_release(&body)?.filter(|v| *v > current))
}

/// Whether a check is due: never checked, a day has passed, or the clock went back.
#[must_use]
pub fn due(last: Option<u64>, now: u64) -> bool {
    last.is_none_or(|last| now < last || now - last >= INTERVAL.as_secs())
}

/// Runs one check if `enabled` and due, recording the attempt in `state_file` first so a
/// failing network is not asked again for a day. Disabled: no file access, no request.
pub async fn check(
    enabled: bool,
    state_file: &Path,
    now: u64,
    client: &reqwest::Client,
    url: &str,
) -> Result<Option<Version>, UpdateError> {
    if !enabled {
        return Ok(None);
    }
    let last = std::fs::read_to_string(state_file)
        .ok()
        .and_then(|t| t.trim().parse().ok());
    if !due(last, now) {
        return Ok(None);
    }
    if let Err(e) = std::fs::write(state_file, now.to_string()) {
        // Without the file the check would run on every start; still at most once a day here.
        tracing::debug!(kind = ?e.kind(), "update check time not saved");
    }
    let Some(current) = Version::current() else {
        return Ok(None);
    };
    latest(client, url, current).await
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    const DAY: u64 = 24 * 60 * 60;

    fn state_file() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "hece-update-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(STATE_FILE)
    }

    async fn github(tag: &str, prerelease: bool) -> MockServer {
        let server = MockServer::start().await;
        let body = serde_json::json!({
            "tag_name": tag,
            "draft": false,
            "prerelease": prerelease,
            "html_url": "https://example.invalid/elsewhere",
        });
        Mock::given(method("GET"))
            .and(path("/releases/latest"))
            .and(header(
                "user-agent",
                concat!("hece/", env!("CARGO_PKG_VERSION")),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;
        server
    }

    fn url(server: &MockServer) -> String {
        format!("{}/releases/latest", server.uri())
    }

    #[test]
    fn versions_compare_and_reject_prereleases() {
        let v = |s| Version::parse(s).unwrap();
        assert!(v("v0.2.0") > v("0.1.9"));
        assert!(v("1.10.0") > v("1.9.3"));
        assert_eq!(v("v1.2.3"), v("1.2.3"));
        for bad in [
            "1.2",
            "1.2.3.4",
            "1.2.3-rc.1",
            "v1.2.3+build",
            "1..3",
            "latest",
        ] {
            assert_eq!(Version::parse(bad), None, "{bad}");
        }
        assert_eq!(
            v("1.2.3").page(),
            "https://github.com/altintasutku/hece/releases/tag/v1.2.3"
        );
    }

    #[test]
    fn checks_at_most_once_a_day() {
        assert!(due(None, 1_000));
        assert!(!due(Some(1_000), 1_000 + DAY - 1));
        assert!(due(Some(1_000), 1_000 + DAY));
        assert!(due(Some(1_000 + DAY), 1_000), "clock went back");
    }

    #[tokio::test]
    async fn newer_release_is_reported_older_and_prerelease_are_not() {
        let current = Version(0, 1, 0);
        let server = github("v0.2.0", false).await;
        let found = latest(&client(), &url(&server), current).await.unwrap();
        assert_eq!(found, Some(Version(0, 2, 0)));
        let server = github("v0.1.0", false).await;
        assert_eq!(
            latest(&client(), &url(&server), current).await.unwrap(),
            None
        );
        let server = github("v9.0.0", true).await;
        assert_eq!(
            latest(&client(), &url(&server), current).await.unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn bad_answers_have_codes() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&server)
            .await;
        let err = latest(&client(), &url(&server), Version(0, 1, 0))
            .await
            .unwrap_err();
        assert_eq!(err.code(), "update_http_error");
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string("<html>"))
            .mount(&server)
            .await;
        let err = latest(&client(), &url(&server), Version(0, 1, 0))
            .await
            .unwrap_err();
        assert_eq!(err.code(), "update_bad_response");
    }

    #[tokio::test]
    async fn second_check_the_same_day_sends_nothing() {
        let server = github("v99.0.0", false).await;
        let file = state_file();
        let found = check(true, &file, 5 * DAY, &client(), &url(&server)).await;
        assert_eq!(found.unwrap(), Some(Version(99, 0, 0)));
        let again = check(true, &file, 5 * DAY + 60, &client(), &url(&server)).await;
        assert_eq!(again.unwrap(), None);
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn disabled_does_nothing() {
        let server = github("v99.0.0", false).await;
        let file = state_file();
        let found = check(false, &file, 5 * DAY, &client(), &url(&server)).await;
        assert_eq!(found.unwrap(), None);
        assert!(server.received_requests().await.unwrap().is_empty());
        assert!(!file.exists());
    }
}
