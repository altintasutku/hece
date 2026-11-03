//! Resumable, checksummed file downloads shared by models and the CUDA runtime. A file is
//! brought to its pinned size (resuming with a `Range` request when the server allows it) and
//! then checked against its pinned sha256; a mismatch deletes it.

use std::io::Write as _;
use std::ops::ControlFlow;
use std::path::Path;
use std::time::Duration;

use sha2::{Digest, Sha256};

/// A stalled transfer fails after this long without data (it resumes on the next try).
const STALL: Duration = Duration::from_secs(30);
/// How often progress is reported while a file downloads, in bytes.
const REPORT_EVERY: u64 = 1 << 20;

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
    #[error("the download failed (network)")]
    Network,
    /// The server answered with an error status.
    #[error("the download server answered {0}")]
    Http(u16),
    /// A file did not match its pinned sha256; it was deleted.
    #[error("{0} did not match its checksum and was deleted")]
    Checksum(String),
    /// A downloaded archive lacks a file it should contain or cannot be read.
    #[error("{0} is not the expected archive")]
    Archive(String),
    /// The disk is full.
    #[error("not enough disk space for the download")]
    DiskFull,
    /// Any other file system error.
    #[error("the download folder could not be written")]
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
            Self::Network => "download_network_error",
            Self::Http(_) => "download_http_error",
            Self::Checksum(_) => "download_checksum_mismatch",
            Self::Archive(_) => "download_archive_invalid",
            Self::DiskFull => "download_disk_full",
            Self::Disk => "download_disk_error",
            Self::Cancelled => "download_cancelled",
        }
    }
}

/// Maps a file system error to [`DownloadError::DiskFull`] or [`DownloadError::Disk`].
pub(crate) fn disk(e: &std::io::Error) -> DownloadError {
    if e.kind() == std::io::ErrorKind::StorageFull {
        DownloadError::DiskFull
    } else {
        DownloadError::Disk
    }
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

/// The HTTP client for downloads: a stall timeout but no overall one (files are large).
#[must_use]
pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(STALL)
        .read_timeout(STALL)
        .build()
        .unwrap_or_default()
}

/// A file pinned to an exact size and sha256.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Pinned<'a> {
    /// Where it is fetched from.
    pub url: &'a str,
    /// Size in bytes.
    pub size: u64,
    /// Lowercase hex sha256.
    pub sha256: &'a str,
}

/// Brings `path` to the pinned file and verifies it; a mismatching file is deleted.
/// `progress` gets the bytes of this file on disk; returning `Break` cancels.
pub(crate) async fn fetch_verified(
    client: &reqwest::Client,
    file: Pinned<'_>,
    path: &Path,
    progress: impl FnMut(u64) -> ControlFlow<()>,
) -> Result<(), DownloadError> {
    fetch(client, file, path, progress).await?;
    if sha256_of(path).map_err(|e| disk(&e))? != file.sha256 {
        let _ = std::fs::remove_file(path);
        let name = path
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        return Err(DownloadError::Checksum(name));
    }
    Ok(())
}

/// Brings `path` to `file.size` bytes, resuming when the server allows it.
async fn fetch(
    client: &reqwest::Client,
    file: Pinned<'_>,
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
    let mut request = client.get(file.url);
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
        out.write_all(&chunk).map_err(|e| disk(&e))?;
        have += chunk.len() as u64;
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
