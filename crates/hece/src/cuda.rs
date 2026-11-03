//! The CUDA runtime the Windows engine needs (`cudart64_13.dll`, `cublas64_13.dll`,
//! `cublasLt64_13.dll`). It is not shipped with Hece: it is downloaded on demand from NVIDIA's
//! own redistributable archives, pinned by URL, size and sha256 like the models, and only the
//! three DLLs (plus NVIDIA's license text) are unpacked into
//! `%LOCALAPPDATA%\hece\runtime\cuda-<version>\`. The engine finds them because that folder is
//! put first on its `PATH`. A CUDA Toolkit already on `PATH` (or DLLs next to the exe) is used
//! as is.
//!
//! The 13.x DLL names are ABI-stable across minor versions; the pin matches the toolkit the
//! engine is built with (13.4.1, from `redistrib_13.4.1.json`).

use std::ffi::OsStr;
use std::io::{Read, Seek};
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};

use crate::download::{DownloadError, Pinned, Progress, disk, fetch_verified};

/// CUDA release the archives come from.
pub const VERSION: &str = "13.4.1";
/// NVIDIA's redistributable archive root; archives are `{REDIST}/{path}`.
pub const REDIST: &str = "https://developer.download.nvidia.com/compute/cuda/redist";
/// The license the downloaded files are under.
pub const EULA: &str = "https://docs.nvidia.com/cuda/eula/index.html";

/// One NVIDIA redistributable archive and the files taken from it.
#[derive(Debug)]
pub struct Archive {
    /// Path under [`REDIST`] (`relative_path` in the manifest).
    pub path: &'static str,
    /// Size in bytes.
    pub size: u64,
    /// Lowercase hex sha256.
    pub sha256: &'static str,
    /// DLLs unpacked from it (matched by file name, wherever they sit in the archive).
    pub dlls: &'static [&'static str],
    /// Name its `LICENSE` file is unpacked as.
    pub license: &'static str,
}

/// The pinned archives (`cuda_cudart` and `libcublas`, `windows-x86_64`).
pub const ARCHIVES: &[Archive] = &[
    Archive {
        path: "cuda_cudart/windows-x86_64/cuda_cudart-windows-x86_64-13.4.49-archive.zip",
        size: 2_736_140,
        sha256: "e6663f3d3e8949eedc2d5ab92c7c5b9fa3f2a222086d91c42bfc5a38bf2b0225",
        dlls: &["cudart64_13.dll"],
        license: "LICENSE-cudart.txt",
    },
    Archive {
        path: "libcublas/windows-x86_64/libcublas-windows-x86_64-13.7.0.27-archive.zip",
        size: 423_620_712,
        sha256: "fff93984ee8a85dd8568e4b9000f9e3ef7153f0f73b176756bcbad3c6622d1f0",
        dlls: &["cublas64_13.dll", "cublasLt64_13.dll"],
        license: "LICENSE-cublas.txt",
    },
];

/// Every DLL the engine needs.
pub fn dlls() -> impl Iterator<Item = &'static str> {
    ARCHIVES.iter().flat_map(|a| a.dlls.iter().copied())
}

/// Total download size in bytes.
#[must_use]
pub fn download_size() -> u64 {
    ARCHIVES.iter().map(|a| a.size).sum()
}

/// Where the runtime is unpacked, under the per-user state folder.
#[must_use]
pub fn runtime_dir(state_dir: &Path) -> PathBuf {
    state_dir.join("runtime").join(format!("cuda-{VERSION}"))
}

/// Whether `dir` holds every DLL.
#[must_use]
pub fn has_dlls(dir: &Path) -> bool {
    dlls().all(|d| dir.join(d).is_file())
}

/// Where a usable runtime was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Location {
    /// The downloaded copy; its folder has to go on the engine's `PATH`.
    Downloaded(PathBuf),
    /// Already reachable: a folder on `PATH` (e.g. the CUDA Toolkit) or next to the exe.
    System(PathBuf),
}

/// Finds the runtime: the downloaded copy first, then the `search` folders in order.
#[must_use]
pub fn locate(downloaded: &Path, search: &[PathBuf]) -> Option<Location> {
    if has_dlls(downloaded) {
        return Some(Location::Downloaded(downloaded.to_path_buf()));
    }
    search
        .iter()
        .find(|d| has_dlls(d))
        .map(|d| Location::System(d.clone()))
}

/// Folders Windows searches for the engine's DLLs besides ours: the exe's folder, then `PATH`.
#[must_use]
pub fn system_dirs(path_var: Option<&OsStr>) -> Vec<PathBuf> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(Path::to_path_buf));
    exe_dir
        .into_iter()
        .chain(
            path_var
                .map(|p| std::env::split_paths(p).collect::<Vec<_>>())
                .unwrap_or_default(),
        )
        .collect()
}

/// Whether an NVIDIA GPU (driver) is present: the driver installs `nvcuda.dll` into
/// `System32`. Only the file's existence is checked; nothing is loaded.
#[must_use]
pub fn nvidia_gpu() -> bool {
    std::env::var_os("SystemRoot")
        .filter(|v| !v.is_empty())
        .is_some_and(|root| {
            PathBuf::from(root)
                .join("System32")
                .join("nvcuda.dll")
                .is_file()
        })
}

/// Why the local engine cannot run. [`RuntimeError::code`] is the stable user-facing code.
#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    /// No NVIDIA GPU and no CUDA runtime: the local engine cannot run here.
    #[error(
        "local transcription needs an NVIDIA GPU (no_nvidia_gpu); pick a cloud provider in Settings"
    )]
    NoGpu,
    /// The GPU is there but the CUDA runtime is not.
    #[error(
        "the CUDA runtime is missing (cuda_runtime_missing); download it in Settings or run `hece runtime download`"
    )]
    Missing,
}

impl RuntimeError {
    /// Stable error code safe to show to users.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::NoGpu => "no_nvidia_gpu",
            Self::Missing => "cuda_runtime_missing",
        }
    }
}

/// The folder to put first on the engine's `PATH`: `Some` for the downloaded copy, `None` when
/// the system already provides the DLLs. Without a runtime this fails; a missing GPU is only
/// reported then, so a working setup is never blocked by the GPU check.
pub fn require(found: Option<Location>, gpu: bool) -> Result<Option<PathBuf>, RuntimeError> {
    match found {
        Some(Location::Downloaded(dir)) => Ok(Some(dir)),
        Some(Location::System(_)) => Ok(None),
        None if gpu => Err(RuntimeError::Missing),
        None => Err(RuntimeError::NoGpu),
    }
}

/// [`locate`] with this user's runtime folder and this process's `PATH`.
pub fn locate_here() -> anyhow::Result<Option<Location>> {
    let downloaded = runtime_dir(&crate::paths::state_dir()?);
    let path = std::env::var_os("PATH");
    Ok(locate(&downloaded, &system_dirs(path.as_deref())))
}

/// The engine's `dll_dir` (see [`require`]). Only the Windows engine needs our runtime; elsewhere
/// the distribution's CUDA packages provide it.
pub fn engine_dll_dir() -> anyhow::Result<Option<PathBuf>> {
    if cfg!(windows) {
        Ok(require(locate_here()?, nvidia_gpu())?)
    } else {
        Ok(None)
    }
}

/// Downloads the pinned archives from `base` and unpacks the DLLs into
/// [`runtime_dir`]`(state_dir)`, which is returned. Archives land in `cuda-<version>.partial`
/// (resumed on the next try); the folder gets its final name only once every DLL is there.
/// `progress` covers the archive bytes; returning `Break` cancels.
pub async fn download(
    client: &reqwest::Client,
    base: &str,
    state_dir: &Path,
    progress: impl FnMut(Progress) -> ControlFlow<()>,
) -> Result<PathBuf, DownloadError> {
    download_archives(client, base, ARCHIVES, state_dir, progress).await
}

async fn download_archives(
    client: &reqwest::Client,
    base: &str,
    archives: &[Archive],
    state_dir: &Path,
    mut progress: impl FnMut(Progress) -> ControlFlow<()>,
) -> Result<PathBuf, DownloadError> {
    let target = runtime_dir(state_dir);
    if has_dlls(&target) {
        return Ok(target);
    }
    let partial = target.with_file_name(format!("cuda-{VERSION}.partial"));
    std::fs::create_dir_all(&partial).map_err(|e| disk(&e))?;
    let total = archives.iter().map(|a| a.size).sum();
    let mut finished = 0;
    let mut zips = Vec::new();
    for archive in archives {
        let name = archive.path.rsplit('/').next().unwrap_or(archive.path);
        let zip = partial.join(name);
        let url = format!("{base}/{}", archive.path);
        let pinned = Pinned {
            url: &url,
            size: archive.size,
            sha256: archive.sha256,
        };
        fetch_verified(client, pinned, &zip, |done| {
            progress(Progress {
                done: finished + done,
                total,
            })
        })
        .await?;
        finished += archive.size;
        zips.push((archive, zip));
    }
    for (archive, zip) in &zips {
        let file = std::fs::File::open(zip).map_err(|e| disk(&e))?;
        extract(file, archive, &partial)?;
    }
    for (_, zip) in &zips {
        std::fs::remove_file(zip).map_err(|e| disk(&e))?;
    }
    // A leftover, incomplete target folder would block the rename.
    if target.exists() {
        std::fs::remove_dir_all(&target).map_err(|e| disk(&e))?;
    }
    std::fs::rename(&partial, &target).map_err(|e| disk(&e))?;
    Ok(target)
}

/// Unpacks only `archive.dlls` and its `LICENSE` into `dest`, by file name. Entry paths are never
/// used as output paths, so a hostile archive cannot write elsewhere.
fn extract(reader: impl Read + Seek, archive: &Archive, dest: &Path) -> Result<(), DownloadError> {
    let name = || {
        archive
            .path
            .rsplit('/')
            .next()
            .unwrap_or(archive.path)
            .to_owned()
    };
    let invalid = |_| DownloadError::Archive(name());
    let mut zip = zip::ZipArchive::new(reader).map_err(invalid)?;
    let mut found = Vec::new();
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(invalid)?;
        if !entry.is_file() {
            continue;
        }
        let entry_name = entry.name().map_err(invalid)?;
        let file_name = entry_name.rsplit(['/', '\\']).next().unwrap_or_default();
        let out = if let Some(dll) = archive.dlls.iter().find(|d| **d == file_name) {
            found.push(*dll);
            *dll
        } else if file_name == "LICENSE" {
            archive.license
        } else {
            continue;
        };
        let mut file = std::fs::File::create(dest.join(out)).map_err(|e| disk(&e))?;
        std::io::copy(&mut entry, &mut file).map_err(|e| {
            if e.kind() == std::io::ErrorKind::InvalidData {
                DownloadError::Archive(name())
            } else {
                disk(&e)
            }
        })?;
    }
    if archive.dlls.iter().all(|d| found.contains(d)) {
        Ok(())
    } else {
        Err(DownloadError::Archive(name()))
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write as _};
    use std::sync::atomic::{AtomicUsize, Ordering};

    use sha2::{Digest, Sha256};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    use zip::write::SimpleFileOptions;

    use super::*;
    use crate::download::hex;

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "hece-cuda-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A synthetic archive laid out like NVIDIA's.
    fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, body) in entries {
            w.start_file(*name, SimpleFileOptions::default()).unwrap();
            w.write_all(body).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    fn put_dlls(dir: &Path) {
        std::fs::create_dir_all(dir).unwrap();
        for d in dlls() {
            std::fs::write(dir.join(d), b"dll").unwrap();
        }
    }

    #[test]
    fn pins_are_complete() {
        for a in ARCHIVES {
            assert_eq!(a.sha256.len(), 64, "{}", a.path);
            assert!(a.size > 0 && !a.dlls.is_empty(), "{}", a.path);
        }
        let all: Vec<_> = dlls().collect();
        assert_eq!(
            all,
            ["cudart64_13.dll", "cublas64_13.dll", "cublasLt64_13.dll"]
        );
    }

    #[test]
    fn extract_takes_only_the_dlls_and_license() {
        let body = zip_of(&[
            ("lib-archive/LICENSE", b"eula"),
            ("lib-archive/bin/x64/cublas64_13.dll", b"a"),
            ("lib-archive/bin/x64/cublasLt64_13.dll", b"b"),
            ("lib-archive/bin/x64/nvblas64_13.dll", b"c"),
            ("lib-archive/include/cublas.h", b"h"),
            ("../../evil/cublas64_13.dll.txt", b"x"),
        ]);
        let dest = temp_dir();
        extract(Cursor::new(body), &ARCHIVES[1], &dest).unwrap();
        let mut names: Vec<_> = std::fs::read_dir(&dest)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(
            names,
            ["LICENSE-cublas.txt", "cublas64_13.dll", "cublasLt64_13.dll"]
        );
        assert_eq!(std::fs::read(dest.join("cublas64_13.dll")).unwrap(), b"a");
    }

    #[test]
    fn extract_fails_when_a_dll_is_missing() {
        let body = zip_of(&[("x/bin/x64/cublas64_13.dll", b"a")]);
        let err = extract(Cursor::new(body), &ARCHIVES[1], &temp_dir()).unwrap_err();
        assert_eq!(err.code(), "download_archive_invalid");
    }

    #[test]
    fn locate_prefers_the_download_then_the_search_path() {
        let root = temp_dir();
        let (ours, toolkit, empty) = (root.join("ours"), root.join("tk"), root.join("empty"));
        put_dlls(&toolkit);
        let search = vec![empty, toolkit.clone()];
        assert_eq!(
            locate(&ours, &search),
            Some(Location::System(toolkit.clone()))
        );
        put_dlls(&ours);
        assert_eq!(
            locate(&ours, &search),
            Some(Location::Downloaded(ours.clone()))
        );
        assert_eq!(locate(&root.join("none"), &[]), None);
    }

    #[test]
    fn require_reports_missing_runtime_with_a_code() {
        let dir = PathBuf::from("rt");
        assert_eq!(
            require(Some(Location::Downloaded(dir.clone())), false).unwrap(),
            Some(dir)
        );
        assert_eq!(
            require(Some(Location::System(PathBuf::from("tk"))), true).unwrap(),
            None
        );
        assert_eq!(
            require(None, true).unwrap_err().code(),
            "cuda_runtime_missing"
        );
        assert_eq!(require(None, false).unwrap_err().code(), "no_nvidia_gpu");
    }

    #[tokio::test]
    async fn download_verifies_unpacks_and_cleans_up() {
        let server = MockServer::start().await;
        let body = zip_of(&[
            ("rt-archive/LICENSE", b"eula"),
            ("rt-archive/bin/x64/cudart64_13.dll", b"rt"),
            ("rt-archive/include/cuda.h", b"h"),
        ]);
        Mock::given(method("GET"))
            .and(path("/rt/rt-archive.zip"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(body.clone()))
            .mount(&server)
            .await;
        let archives = [Archive {
            path: "rt/rt-archive.zip",
            size: body.len() as u64,
            sha256: Box::leak(hex(&Sha256::digest(&body)).into_boxed_str()),
            dlls: &["cudart64_13.dll"],
            license: "LICENSE-cudart.txt",
        }];
        let state = temp_dir();
        let got = download_archives(
            &crate::download::client(),
            &server.uri(),
            &archives,
            &state,
            |_| ControlFlow::Continue(()),
        )
        .await
        .unwrap();
        assert_eq!(got, runtime_dir(&state));
        let mut names: Vec<_> = std::fs::read_dir(&got)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(names, ["LICENSE-cudart.txt", "cudart64_13.dll"]);
        assert!(
            !got.with_file_name(format!("cuda-{VERSION}.partial"))
                .exists()
        );
    }
}
