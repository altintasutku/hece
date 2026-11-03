//! Pipe names for the daemon and engine pipes. Named pipes share one machine-wide namespace, so
//! the name mixes in a random per-user key kept next to the socket path (`pipe-key` in
//! `%LOCALAPPDATA%\hece`, which inherits the profile's per-user ACL). The user SID would not
//! help: other accounts can look it up, and the path already names the user.

use std::io::{self, ErrorKind, Write as _};
use std::path::Path;

/// File holding the key, next to the socket path.
const KEY_FILE: &str = "pipe-key";
/// Random bytes in a key (hex encoded in the file).
const KEY_BYTES: usize = 16;

/// The named pipe for `socket` (see [`hece_core::ipc::pipe_name`]), creating the per-user key
/// on first use. Daemon, CLI, settings app and engine all derive the name through this.
pub fn pipe_name(socket: &Path) -> io::Result<String> {
    let dir = socket.parent().unwrap_or(Path::new(""));
    Ok(hece_core::ipc::pipe_name(socket, &key(dir)?))
}

fn valid(key: &str) -> bool {
    key.len() == KEY_BYTES * 2 && key.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Reads the key in `dir`, writing a new one when it is missing or malformed. The file is
/// written under a temporary name and then linked into place, so a concurrent reader never sees
/// a partial key and two first users agree on one.
fn key(dir: &Path) -> io::Result<String> {
    let path = dir.join(KEY_FILE);
    let existing = match std::fs::read_to_string(&path) {
        Ok(s) if valid(s.trim()) => return Ok(s.trim().to_owned()),
        Ok(_) => true,
        Err(e) if e.kind() == ErrorKind::NotFound => false,
        Err(e) => return Err(e),
    };
    let mut bytes = [0u8; KEY_BYTES];
    getrandom::fill(&mut bytes).map_err(|_| io::Error::other("no randomness"))?;
    let key = bytes.iter().fold(String::new(), |mut s, b| {
        let _ = std::fmt::Write::write_fmt(&mut s, format_args!("{b:02x}"));
        s
    });
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!("{KEY_FILE}.{}.tmp", std::process::id()));
    std::fs::File::create(&tmp)?.write_all(key.as_bytes())?;
    let placed = if existing {
        std::fs::rename(&tmp, &path)
    } else {
        std::fs::hard_link(&tmp, &path)
    };
    let _ = std::fs::remove_file(&tmp);
    match placed {
        Ok(()) => Ok(key),
        // Another process created it first; use theirs.
        Err(e) if e.kind() == ErrorKind::AlreadyExists => {
            let theirs = std::fs::read_to_string(&path)?;
            let theirs = theirs.trim();
            if valid(theirs) {
                Ok(theirs.to_owned())
            } else {
                Err(io::Error::new(ErrorKind::InvalidData, "malformed pipe key"))
            }
        }
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_is_created_once_and_reused() {
        let dir = std::env::temp_dir().join(format!("hece-pipe-key-{}", std::process::id()));
        let socket = dir.join("engine.sock");
        let name = pipe_name(&socket).unwrap();
        assert_eq!(name, pipe_name(&socket).unwrap());
        let key = std::fs::read_to_string(dir.join(KEY_FILE)).unwrap();
        assert!(valid(&key));
        assert_eq!(name, hece_core::ipc::pipe_name(&socket, &key));
        // A damaged key is replaced, which changes the name.
        std::fs::write(dir.join(KEY_FILE), "x").unwrap();
        assert_ne!(name, pipe_name(&socket).unwrap());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
