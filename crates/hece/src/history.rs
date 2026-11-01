//! Dictation history for `hece history` and `hece repaste`: the last N entries within D days,
//! one JSON object per line in the per-user state directory. Can be disabled in the config.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::config::HistoryConfig;

/// One dictation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    /// Unix time in seconds.
    pub time: u64,
    /// Transcript after the quality layers.
    pub raw: String,
    /// Text after LLM cleanup, when it ran.
    pub cleaned: Option<String>,
    pub provider: String,
    pub low_confidence: bool,
}

impl Entry {
    /// The text that was inserted.
    pub fn text(&self) -> &str {
        self.cleaned.as_deref().unwrap_or(&self.raw)
    }
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[derive(Debug, Clone)]
pub struct History {
    path: PathBuf,
    max_entries: usize,
    max_age_secs: u64,
}

impl History {
    pub fn new(state_dir: &Path, cfg: &HistoryConfig) -> Self {
        Self {
            path: state_dir.join("history.jsonl"),
            max_entries: cfg.max_entries,
            max_age_secs: cfg.max_days.saturating_mul(86_400),
        }
    }

    /// Entries still within the limits, oldest first. Unreadable lines are skipped.
    pub fn entries(&self) -> Vec<Entry> {
        let Ok(text) = std::fs::read_to_string(&self.path) else {
            return Vec::new();
        };
        let cutoff = now().saturating_sub(self.max_age_secs);
        let mut entries: Vec<Entry> = text
            .lines()
            .filter_map(|l| serde_json::from_str::<Entry>(l).ok())
            .filter(|e| e.time >= cutoff)
            .collect();
        let excess = entries.len().saturating_sub(self.max_entries);
        entries.drain(..excess);
        entries
    }

    /// Adds `entry` and drops what falls out of the limits. The file is replaced atomically.
    #[cfg_attr(
        all(not(windows), not(test)),
        expect(dead_code, reason = "written by the Windows-only daemon")
    )]
    pub fn append(&self, entry: Entry) -> std::io::Result<()> {
        let mut entries = self.entries();
        entries.push(entry);
        let excess = entries.len().saturating_sub(self.max_entries);
        entries.drain(..excess);
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = self.path.with_extension("jsonl.tmp");
        let mut file = private_file(&tmp)?;
        for e in &entries {
            serde_json::to_writer(&mut file, e)?;
            file.write_all(b"\n")?;
        }
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, &self.path)
    }

    /// Deletes the history file.
    pub fn clear(&self) -> std::io::Result<()> {
        match std::fs::remove_file(&self.path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            other => other,
        }
    }
}

/// Creates `path` readable by its owner only (0600 on unix; on Windows the state directory's
/// per-user ACL applies).
#[cfg_attr(
    all(not(windows), not(test)),
    expect(dead_code, reason = "written by the Windows-only daemon")
)]
fn private_file(path: &Path) -> std::io::Result<std::fs::File> {
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    opts.open(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(time: u64, raw: &str) -> Entry {
        Entry {
            time,
            raw: raw.to_owned(),
            cleaned: None,
            provider: "local".to_owned(),
            low_confidence: false,
        }
    }

    fn history(dir: &Path, max_entries: usize, max_days: u64) -> History {
        History::new(
            dir,
            &HistoryConfig {
                enabled: true,
                max_entries,
                max_days,
            },
        )
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("hece-history-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn keeps_the_newest_entries_within_count_and_age() {
        let dir = temp_dir("limits");
        let h = history(&dir, 2, 7);
        h.append(entry(1, "too old")).unwrap();
        for text in ["a", "b", "c"] {
            h.append(entry(now(), text)).unwrap();
        }
        let texts: Vec<String> = h.entries().into_iter().map(|e| e.raw).collect();
        assert_eq!(texts, ["b", "c"]);
        h.clear().unwrap();
        assert_eq!(h.entries(), Vec::<Entry>::new());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
