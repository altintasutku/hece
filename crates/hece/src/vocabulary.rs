//! The vocabulary file as the settings window edits it: one term per line, earlier terms first.
//! `vocabulary_file` in the config wins; otherwise `vocabulary.txt` next to `config.toml`, created
//! on the first save. Saving replaces the term lines and keeps comments, blank lines and `!term`
//! exclusions where they were. Terms are personal data: never logged, never in error messages.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use hece_core::vocab::{DEFAULT_PROMPT_PREFIX, Vocabulary, clean_term, entry};
use hece_providers::vocab_hint::estimate_tokens;
use serde::Serialize;

/// The default file name, in the config folder.
pub const FILE_NAME: &str = "vocabulary.txt";

/// The first lines of a file Hece creates.
const HEADER: &str = "# Hece vocabulary: one term per line, earlier terms have priority.\n\
                      # Lines starting with # are kept; !term leaves a term out.\n";

/// The file in use: `explicit` (`[transcription] vocabulary_file`), else the default.
#[must_use]
pub fn path(explicit: Option<&Path>, config_file: &Path) -> PathBuf {
    explicit.map_or_else(|| config_file.with_file_name(FILE_NAME), Path::to_path_buf)
}

/// The terms of a file in file order, as written (bullet stripped), valid or not.
#[must_use]
pub fn entries(text: &str) -> Vec<String> {
    text.lines().filter_map(entry).map(str::to_owned).collect()
}

/// One line per entry: line breaks and control characters become spaces.
fn one_line(term: &str) -> String {
    term.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .trim()
        .to_owned()
}

/// `text` with its term lines replaced by `terms` in order. Lines that are not terms stay where
/// they are; extra terms go at the end, missing ones drop their lines. Blank terms are skipped.
#[must_use]
pub fn merge(text: &str, terms: &[String]) -> String {
    let mut terms = terms.iter().map(|t| one_line(t)).filter(|t| !t.is_empty());
    let mut out = String::with_capacity(text.len());
    if text.trim().is_empty() {
        out.push_str(HEADER);
    }
    for line in text.lines() {
        if entry(line).is_none() {
            out.push_str(line);
            out.push('\n');
        } else if let Some(term) = terms.next() {
            out.push_str(&term);
            out.push('\n');
        }
    }
    for term in terms {
        out.push_str(&term);
        out.push('\n');
    }
    out
}

/// How an entry is used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// A term Hece uses.
    Ok,
    /// Same as an earlier term (case-insensitive); skipped.
    Duplicate,
    /// Empty, too long, with a comma, without letters, or starting with `#` or `!`; skipped.
    Invalid,
}

/// What the window shows next to the list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Check {
    /// One per entry.
    pub statuses: Vec<Status>,
    /// Usable terms.
    pub terms: usize,
    /// Usable terms that fit the prompt budget (estimated, so at least this many).
    pub fit: usize,
}

/// Checks `entries` the way the daemon reads them, against a prompt of `prompt_tokens`.
#[must_use]
pub fn check(entries: &[String], prompt_tokens: usize) -> Check {
    let mut seen = HashSet::new();
    let mut terms = Vec::new();
    let statuses = entries
        .iter()
        .map(|raw| {
            let raw = one_line(raw);
            match (raw.starts_with(['#', '!']), clean_term(&raw)) {
                (false, Some(term)) => {
                    if seen.insert(hece_core::fold(&term)) {
                        terms.push(term);
                        Status::Ok
                    } else {
                        Status::Duplicate
                    }
                }
                _ => Status::Invalid,
            }
        })
        .collect();
    let vocab = Vocabulary { terms };
    Check {
        statuses,
        terms: vocab.terms.len(),
        fit: vocab.fitting_terms(DEFAULT_PROMPT_PREFIX, &estimate_tokens, prompt_tokens),
    }
}

/// Problems reading or writing the file. Messages carry the path, never terms.
#[derive(Debug, thiserror::Error)]
pub enum VocabularyError {
    /// The file exists but cannot be read.
    #[error("cannot read {}: {kind:?}", path.display())]
    Read {
        /// The vocabulary file.
        path: PathBuf,
        /// Why.
        kind: std::io::ErrorKind,
    },
    /// The file or its folder cannot be written.
    #[error("cannot write {}: {kind:?}", path.display())]
    Write {
        /// The vocabulary file.
        path: PathBuf,
        /// Why.
        kind: std::io::ErrorKind,
    },
}

impl VocabularyError {
    /// Stable error code safe to show to users.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::Read { .. } => "vocabulary_unreadable",
            Self::Write { .. } => "vocabulary_write_failed",
        }
    }
}

/// The file's text; a missing file is empty.
pub fn read(path: &Path) -> Result<String, VocabularyError> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(VocabularyError::Read {
            path: path.to_owned(),
            kind: e.kind(),
        }),
    }
}

/// Writes `terms` into the file at `path` (created with its folder if missing), keeping its
/// comments.
pub fn save(path: &Path, terms: &[String]) -> Result<(), VocabularyError> {
    let text = merge(&read(path)?, terms);
    let failed = |e: std::io::Error| VocabularyError::Write {
        path: path.to_owned(),
        kind: e.kind(),
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(failed)?;
    }
    let tmp = path.with_extension("txt.tmp");
    std::fs::write(&tmp, text)
        .and_then(|()| std::fs::rename(&tmp, path))
        .map_err(failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terms(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn default_path_sits_next_to_the_config() {
        let config = Path::new("C:/Users/you/AppData/Roaming/hece/config.toml");
        assert_eq!(
            path(None, config),
            Path::new("C:/Users/you/AppData/Roaming/hece/vocabulary.txt")
        );
        let own = Path::new("D:/notes/terms.txt");
        assert_eq!(path(Some(own), config), own);
    }

    #[test]
    fn merge_keeps_comments_and_exclusions_in_place() {
        let text = "# names\n- Alpha\nBravo\n\n# tools\n!Charlie\nDelta\n";
        assert_eq!(entries(text), ["Alpha", "Bravo", "Delta"]);
        // Reordered, one removed, one added.
        let merged = merge(text, &terms(&["Delta", "Alpha", "Echo", "Foxtrot"]));
        assert_eq!(
            merged,
            "# names\nDelta\nAlpha\n\n# tools\n!Charlie\nEcho\nFoxtrot\n"
        );
        assert_eq!(
            merge(text, &terms(&["Alpha"])),
            "# names\nAlpha\n\n# tools\n!Charlie\n"
        );
    }

    #[test]
    fn new_file_gets_a_header_and_blank_or_multiline_terms_are_tidied() {
        let merged = merge("", &terms(&["  Alpha ", "", "Bra\nvo"]));
        assert_eq!(merged, format!("{HEADER}Alpha\nBra vo\n"));
        assert_eq!(entries(&merged), ["Alpha", "Bra vo"]);
    }

    #[test]
    fn check_flags_skipped_entries() {
        use Status::{Duplicate, Invalid, Ok};
        let long = "x".repeat(41);
        let entries = terms(&["Alpha", "alpha", "a, b", "#tag", "...", &long, "Bravo"]);
        let result = check(&entries, 200);
        assert_eq!(
            result.statuses,
            [Ok, Duplicate, Invalid, Invalid, Invalid, Invalid, Ok]
        );
        assert_eq!((result.terms, result.fit), (2, 2));
        assert_eq!(check(&entries, 5).fit, 0);
    }

    #[test]
    fn save_creates_the_file_and_round_trips() {
        let dir = std::env::temp_dir().join(format!("hece-vocab-{}", std::process::id()));
        let file = dir.join("sub").join(FILE_NAME);
        save(&file, &terms(&["Alpha", "Bravo"])).unwrap();
        save(&file, &terms(&["Bravo", "Alpha"])).unwrap();
        let text = read(&file).unwrap();
        assert!(text.starts_with(HEADER));
        assert_eq!(entries(&text), ["Bravo", "Alpha"]);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
