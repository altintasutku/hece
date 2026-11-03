//! Local models: a catalog pinned to exact Hugging Face revisions, and a resumable, checksummed
//! download into `%LOCALAPPDATA%\hece\models\<id>`. Files land in `<id>.partial` first; the folder
//! is renamed only once every file matches its size and sha256, so a half-downloaded model is
//! never picked up.

use std::ops::ControlFlow;
use std::path::{Path, PathBuf};

pub use crate::download::{DownloadError, Progress, client};
use crate::download::{Pinned, disk, fetch_verified};

/// Where catalog files are fetched from: `{HUB}/{repo}/resolve/{revision}/{file}`.
pub const HUB: &str = "https://huggingface.co";

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
        let url = format!(
            "{hub}/{}/resolve/{}/{}",
            model.repo, model.revision, file.name
        );
        let pinned = Pinned {
            url: &url,
            size: file.size,
            sha256: file.sha256,
        };
        fetch_verified(client, pinned, &partial.join(file.name), |done| {
            progress(Progress {
                done: finished + done,
                total,
            })
        })
        .await?;
        finished += file.size;
    }
    // A leftover, incomplete target folder would block the rename.
    if target.exists() {
        std::fs::remove_dir_all(&target).map_err(|e| disk(&e))?;
    }
    std::fs::rename(&partial, &target).map_err(|e| disk(&e))?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use sha2::{Digest, Sha256};
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::download::hex;

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
        assert_eq!(err.code(), "download_checksum_mismatch");
        assert!(!dir.join("test-model.partial").join("model.bin").exists());
        assert!(!installed(&m, &dir));
    }

    #[tokio::test]
    async fn http_error_has_a_code() {
        let server = MockServer::start().await;
        let m = model(vec![file("model.bin", b"x")]);
        let err = download(&client(), &server.uri(), &m, &temp_dir(), keep_going)
            .await
            .unwrap_err();
        assert_eq!(err.code(), "download_http_error");
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
        assert_eq!(err.code(), "download_cancelled");
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
