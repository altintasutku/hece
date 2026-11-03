//! Local models: a catalog pinned to exact Hugging Face revisions, and a resumable, checksummed
//! download into `%LOCALAPPDATA%\hece\models\<id>`. Files land in `<id>.partial` first; the folder
//! is renamed only once every file matches its size and sha256, so a half-downloaded model is
//! never picked up.

use std::io::Write as _;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::time::Duration;

use sha2::{Digest, Sha256};

/// Where catalog files are fetched from: `{HUB}/{repo}/resolve/{revision}/{file}`.
pub const HUB: &str = "https://huggingface.co";
/// A stalled transfer fails after this long without data (it resumes on the next try).
const STALL: Duration = Duration::from_secs(30);
/// How often progress is reported while a file downloads, in bytes.
const REPORT_EVERY: u64 = 1 << 20;

/// One file of a model.
#[derive(Debug)]
pub struct ModelFile {
    /// File name in the repo and on disk.
    pub name: &'static str,
    /// Size in bytes.
    pub size: u64,
    /// Lowercase hex sha256.
    pub sha256: &'static str,
}

/// A downloadable model.
#[derive(Debug)]
pub struct Model {
    /// Folder name under the models directory, and the id users pick.
    pub id: &'static str,
    /// Display name.
    pub name: &'static str,
    /// Hugging Face repo.
    pub repo: &'static str,
    /// Commit the files are pinned to.
    pub revision: &'static str,
    /// Languages it is meant for; empty means many.
    pub languages: &'static [&'static str],
    /// Every file the engine needs.
    pub files: &'static [ModelFile],
}

impl Model {
    /// Total download size in bytes.
    #[must_use]
    pub fn size(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }
}

/// The models Hece can download. The first one is the default.
pub const CATALOG: &[Model] = &[TURBO, TURKISH];

const TURKISH: Model = Model {
    id: "large-v3-turkish-ec03ccf-int8",
    name: "Turkish (Whisper large-v3, fine-tuned)",
    repo: "altintasutku/whisper-large-v3-turkish-ct2-int8",
    revision: "ed12e3844616ad7fa77b79ea44f92f8dfdbecf99",
    languages: &["tr"],
    files: &[
        ModelFile {
            name: "model.bin",
            size: 1_551_326_177,
            sha256: "b81d047036f37088f869b09a65aacd89099572a9731b73527c7e5ba4aa307740",
        },
        ModelFile {
            name: "config.json",
            size: 2_634,
            sha256: "f277d9390e65adc1630b349151eca38aaf7420c41c814271928854edcfd6ffbe",
        },
        ModelFile {
            name: "tokenizer.json",
            size: 2_480_617,
            sha256: "6d8cbd7cd0d8d5815e478dac67b85a26bbe77c1f5e0c6d76d1ce2abc0e5f21ca",
        },
        ModelFile {
            name: "vocabulary.json",
            size: 1_119_981,
            sha256: "697ee5a65726e46b4d294d1a243d98e1878f5d81caba17891b1c5ebab7a912a9",
        },
        ModelFile {
            name: "preprocessor_config.json",
            size: 340,
            sha256: "7ccc62c6f2765af1f3b46c00c9b5894426835a05021c8b9c01eecb6dfb542711",
        },
    ],
};

const TURBO: Model = Model {
    id: "large-v3-turbo-0a363e9",
    name: "Multilingual (Whisper large-v3 turbo)",
    repo: "dropbox-dash/faster-whisper-large-v3-turbo",
    revision: "0a363e9161cbc7ed1431c9597a8ceaf0c4f78fcf",
    languages: &[],
    files: &[
        ModelFile {
            name: "model.bin",
            size: 1_617_884_929,
            sha256: "e76620f83d5f5b69efd3d87e3dc180c1bd21df9fbebacfd4335e5e1efcc018da",
        },
        ModelFile {
            name: "config.json",
            size: 2_263,
            sha256: "b0253ea6c0d3bea6b1e19e91a02acfd3b53f4467362efcb5a3e6b16c9b3a9b7e",
        },
        ModelFile {
            name: "tokenizer.json",
            size: 2_710_337,
            sha256: "297b13372ac43916285644fb9687add3cc62ee2a1adb60da3dc25cc94c1871fd",
        },
        ModelFile {
            name: "vocabulary.json",
            size: 1_068_114,
            sha256: "c69260f2ab26d659b7c398f9a2b2b48ed0df16c3b47d7326782fd9cba71690c1",
        },
        ModelFile {
            name: "preprocessor_config.json",
            size: 340,
            sha256: "7ccc62c6f2765af1f3b46c00c9b5894426835a05021c8b9c01eecb6dfb542711",
        },
    ],
};

/// The catalog entry with this id.
#[must_use]
pub fn find(id: &str) -> Option<&'static Model> {
    CATALOG.iter().find(|m| m.id == id)
}

/// How far a download is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    /// Bytes on disk so far, over all files.
    pub done: u64,
    /// Bytes in total.
    pub total: u64,
}

/// Download failures. [`DownloadError::code`] is the stable user-facing code.
#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    /// The server could not be reached or the transfer broke off.
    #[error("the model could not be downloaded (network)")]
    Network,
    /// The server answered with an error status.
    #[error("the model server answered {0}")]
    Http(u16),
    /// A file did not match its pinned sha256; it was deleted.
    #[error("{0} did not match its checksum and was deleted")]
    Checksum(&'static str),
    /// The disk is full.
    #[error("not enough disk space for the model")]
    DiskFull,
    /// Any other file system error.
    #[error("the model folder could not be written")]
    Disk,
    /// The download was cancelled; what arrived is kept for the next try.
    #[error("the download was cancelled")]
    Cancelled,
}

impl DownloadError {
    /// Stable error code safe to show to users.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::Network => "model_network_error",
            Self::Http(_) => "model_http_error",
            Self::Checksum(_) => "model_checksum_mismatch",
            Self::DiskFull => "model_disk_full",
            Self::Disk => "model_disk_error",
            Self::Cancelled => "model_download_cancelled",
        }
    }
}

fn disk(e: &std::io::Error) -> DownloadError {
    if e.kind() == std::io::ErrorKind::StorageFull {
        DownloadError::DiskFull
    } else {
        DownloadError::Disk
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

/// The HTTP client for downloads: a stall timeout but no overall one (files are large). HTTPS
/// only, also for redirects (the hub sends files to its CDN); unit tests serve plain HTTP.
#[must_use]
pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .https_only(!cfg!(test))
        .connect_timeout(STALL)
        .read_timeout(STALL)
        .build()
        .unwrap_or_default()
}

/// Where downloaded models live (`%LOCALAPPDATA%\hece\models`).
pub fn models_dir() -> anyhow::Result<PathBuf> {
    Ok(crate::paths::state_dir()?.join("models"))
}

/// Whether `model` is fully downloaded under `models_dir` (sizes only; checksums were verified
/// before the folder got its final name).
#[must_use]
pub fn installed(model: &Model, models_dir: &Path) -> bool {
    let dir = models_dir.join(model.id);
    model.files.iter().all(|f| {
        std::fs::metadata(dir.join(f.name)).is_ok_and(|m| m.is_file() && m.len() == f.size)
    })
}

/// Downloads `model` from `hub` into `models_dir/<id>` and returns that folder. Resumes from
/// what an earlier attempt left in `<id>.partial`. `progress` is called as data arrives; returning
/// `Break` cancels.
pub async fn download(
    client: &reqwest::Client,
    hub: &str,
    model: &Model,
    models_dir: &Path,
    mut progress: impl FnMut(Progress) -> ControlFlow<()>,
) -> Result<PathBuf, DownloadError> {
    let target = models_dir.join(model.id);
    if installed(model, models_dir) {
        return Ok(target);
    }
    let partial = models_dir.join(format!("{}.partial", model.id));
    std::fs::create_dir_all(&partial).map_err(|e| disk(&e))?;
    let total = model.size();
    let mut finished = 0;
    for file in model.files {
        let path = partial.join(file.name);
        fetch(client, hub, model, file, &path, |done| {
            progress(Progress {
                done: finished + done,
                total,
            })
        })
        .await?;
        if sha256_of(&path).map_err(|e| disk(&e))? != file.sha256 {
            let _ = std::fs::remove_file(&path);
            return Err(DownloadError::Checksum(file.name));
        }
        finished += file.size;
    }
    // A leftover, incomplete target folder would block the rename.
    if target.exists() {
        std::fs::remove_dir_all(&target).map_err(|e| disk(&e))?;
    }
    std::fs::rename(&partial, &target).map_err(|e| disk(&e))?;
    Ok(target)
}

/// Brings `path` to `file.size` bytes from the hub, resuming when the server allows it.
async fn fetch(
    client: &reqwest::Client,
    hub: &str,
    model: &Model,
    file: &ModelFile,
    path: &Path,
    mut progress: impl FnMut(u64) -> ControlFlow<()>,
) -> Result<(), DownloadError> {
    let mut have = std::fs::metadata(path).map_or(0, |m| m.len());
    if have > file.size {
        std::fs::remove_file(path).map_err(|e| disk(&e))?;
        have = 0;
    }
    if have == file.size {
        return Ok(());
    }
    let url = format!(
        "{hub}/{}/resolve/{}/{}",
        model.repo, model.revision, file.name
    );
    let mut request = client.get(&url);
    if have > 0 {
        request = request.header(reqwest::header::RANGE, format!("bytes={have}-"));
    }
    let mut response = request.send().await.map_err(|_| DownloadError::Network)?;
    let status = response.status();
    if !status.is_success() {
        return Err(DownloadError::Http(status.as_u16()));
    }
    let resumed = status == reqwest::StatusCode::PARTIAL_CONTENT
        && response
            .headers()
            .get(reqwest::header::CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with(&format!("bytes {have}-")));
    let opened = if resumed {
        std::fs::OpenOptions::new().append(true).open(path)
    } else {
        // A full answer (or a range we did not ask for) starts the file over.
        have = 0;
        std::fs::File::create(path)
    };
    let mut out = opened.map_err(|e| disk(&e))?;
    let mut reported = have;
    if progress(have).is_break() {
        return Err(DownloadError::Cancelled);
    }
    while let Some(chunk) = response.chunk().await.map_err(|_| DownloadError::Network)? {
        have += chunk.len() as u64;
        // More than the pinned size can never match the checksum; stop before it fills the disk.
        if have > file.size {
            drop(out);
            let _ = std::fs::remove_file(path);
            return Err(DownloadError::Checksum(file.name));
        }
        out.write_all(&chunk).map_err(|e| disk(&e))?;
        if have - reported >= REPORT_EVERY || have == file.size {
            reported = have;
            if progress(have).is_break() {
                return Err(DownloadError::Cancelled);
            }
        }
    }
    out.flush().map_err(|e| disk(&e))?;
    Ok(())
}

fn sha256_of(path: &Path) -> std::io::Result<String> {
    let mut hasher = Sha256::new();
    let mut file = std::fs::File::open(path)?;
    std::io::copy(&mut file, &mut hasher)?;
    Ok(hex(&hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use sha2::{Digest, Sha256};
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "hece-models-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn leak(s: String) -> &'static str {
        Box::leak(s.into_boxed_str())
    }

    fn file(name: &'static str, body: &[u8]) -> ModelFile {
        ModelFile {
            name,
            size: body.len() as u64,
            sha256: leak(hex(&Sha256::digest(body))),
        }
    }

    fn model(files: Vec<ModelFile>) -> Model {
        Model {
            id: "test-model",
            name: "Test",
            repo: "owner/test",
            revision: "abc123",
            languages: &["tr"],
            files: Box::leak(files.into_boxed_slice()),
        }
    }

    fn url(name: &str) -> String {
        format!("/owner/test/resolve/abc123/{name}")
    }

    async fn serve(server: &MockServer, name: &str, body: &[u8]) {
        Mock::given(method("GET"))
            .and(path(url(name)))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(body.to_vec()))
            .mount(server)
            .await;
    }

    fn keep_going(_: Progress) -> ControlFlow<()> {
        ControlFlow::Continue(())
    }

    #[tokio::test]
    async fn downloads_and_verifies_into_final_dir() {
        let server = MockServer::start().await;
        let (a, b) = (b"model weights".as_slice(), b"{\"x\":1}".as_slice());
        serve(&server, "model.bin", a).await;
        serve(&server, "config.json", b).await;
        let m = model(vec![file("model.bin", a), file("config.json", b)]);
        let dir = temp_dir();
        let got = download(&client(), &server.uri(), &m, &dir, keep_going)
            .await
            .unwrap();
        assert_eq!(got, dir.join("test-model"));
        assert_eq!(std::fs::read(got.join("model.bin")).unwrap(), a);
        assert_eq!(std::fs::read(got.join("config.json")).unwrap(), b);
        assert!(!dir.join("test-model.partial").exists());
        assert!(installed(&m, &dir));
    }

    #[tokio::test]
    async fn resumes_partial_file_with_range() {
        let server = MockServer::start().await;
        let body = b"0123456789".as_slice();
        Mock::given(method("GET"))
            .and(path(url("model.bin")))
            .and(header("range", "bytes=4-"))
            .respond_with(
                ResponseTemplate::new(206)
                    .insert_header("content-range", "bytes 4-9/10")
                    .set_body_bytes(body[4..].to_vec()),
            )
            .expect(1)
            .mount(&server)
            .await;
        let m = model(vec![file("model.bin", body)]);
        let dir = temp_dir();
        std::fs::create_dir_all(dir.join("test-model.partial")).unwrap();
        std::fs::write(dir.join("test-model.partial").join("model.bin"), &body[..4]).unwrap();
        let got = download(&client(), &server.uri(), &m, &dir, keep_going)
            .await
            .unwrap();
        assert_eq!(std::fs::read(got.join("model.bin")).unwrap(), body);
    }

    #[tokio::test]
    async fn restarts_when_server_ignores_range() {
        let server = MockServer::start().await;
        let body = b"0123456789".as_slice();
        serve(&server, "model.bin", body).await;
        let m = model(vec![file("model.bin", body)]);
        let dir = temp_dir();
        std::fs::create_dir_all(dir.join("test-model.partial")).unwrap();
        std::fs::write(dir.join("test-model.partial").join("model.bin"), b"0123").unwrap();
        let got = download(&client(), &server.uri(), &m, &dir, keep_going)
            .await
            .unwrap();
        assert_eq!(std::fs::read(got.join("model.bin")).unwrap(), body);
    }

    #[tokio::test]
    async fn checksum_mismatch_fails_and_drops_the_file() {
        let server = MockServer::start().await;
        serve(&server, "model.bin", b"tampered!!").await;
        let m = model(vec![file("model.bin", b"0123456789")]);
        let dir = temp_dir();
        let err = download(&client(), &server.uri(), &m, &dir, keep_going)
            .await
            .unwrap_err();
        assert_eq!(err.code(), "model_checksum_mismatch");
        assert!(!dir.join("test-model.partial").join("model.bin").exists());
        assert!(!installed(&m, &dir));
    }

    #[tokio::test]
    async fn oversized_answer_stops_at_the_pinned_size() {
        let server = MockServer::start().await;
        serve(&server, "model.bin", &[0_u8; 4096]).await;
        let m = model(vec![file("model.bin", b"0123456789")]);
        let dir = temp_dir();
        let err = download(&client(), &server.uri(), &m, &dir, keep_going)
            .await
            .unwrap_err();
        assert_eq!(err.code(), "model_checksum_mismatch");
        assert!(!dir.join("test-model.partial").join("model.bin").exists());
    }

    #[tokio::test]
    async fn http_error_has_a_code() {
        let server = MockServer::start().await;
        let m = model(vec![file("model.bin", b"x")]);
        let err = download(&client(), &server.uri(), &m, &temp_dir(), keep_going)
            .await
            .unwrap_err();
        assert_eq!(err.code(), "model_http_error");
    }

    #[tokio::test]
    async fn cancel_stops_and_keeps_the_partial() {
        let server = MockServer::start().await;
        let body = vec![7_u8; 64 * 1024];
        serve(&server, "model.bin", &body).await;
        let m = model(vec![file("model.bin", &body)]);
        let dir = temp_dir();
        let err = download(&client(), &server.uri(), &m, &dir, |_| {
            ControlFlow::Break(())
        })
        .await
        .unwrap_err();
        assert_eq!(err.code(), "model_download_cancelled");
        assert!(!installed(&m, &dir));
    }

    #[test]
    fn catalog_entries_are_pinned_and_complete() {
        for m in CATALOG {
            assert_eq!(m.revision.len(), 40, "{}", m.id);
            for f in m.files {
                assert_eq!(f.sha256.len(), 64, "{} {}", m.id, f.name);
                assert!(f.size > 0);
            }
            for required in ["model.bin", "tokenizer.json", "config.json"] {
                assert!(m.files.iter().any(|f| f.name == required), "{}", m.id);
            }
        }
    }
}
