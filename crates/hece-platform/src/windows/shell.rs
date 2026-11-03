//! Desktop notifications and autostart.

use std::os::windows::process::CommandExt as _;
use std::path::Path;
use std::process::Command;

use tauri_winrt_notification::{Sound, Toast};

use crate::PlatformError;

const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "Hece";
/// `CREATE_NO_WINDOW`: no console flashes when the windowless daemon runs `reg.exe`.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Shows a toast. Never carries transcript text. Blocks briefly; best effort.
pub fn notify(title: &str, body: &str) {
    let shown = Toast::new(Toast::POWERSHELL_APP_ID)
        .title(title)
        .text1(body)
        .sound(None::<Sound>)
        .show();
    if shown.is_err() {
        tracing::debug!("toast could not be shown");
    }
}

fn reg(args: &[&str]) -> Result<bool, PlatformError> {
    Command::new("reg.exe")
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map(|o| o.status.success())
        .map_err(|_| PlatformError::Autostart)
}

/// Starts `exe` at login (`HKCU\...\Run`).
pub fn enable_autostart(exe: &Path) -> Result<(), PlatformError> {
    let command = format!("\"{}\"", exe.display());
    let ok = reg(&[
        "add", RUN_KEY, "/v", RUN_VALUE, "/t", "REG_SZ", "/d", &command, "/f",
    ])?;
    if ok {
        Ok(())
    } else {
        Err(PlatformError::Autostart)
    }
}

/// Stops starting at login; succeeds when it was not enabled.
pub fn disable_autostart() -> Result<(), PlatformError> {
    if autostart_enabled()? {
        let ok = reg(&["delete", RUN_KEY, "/v", RUN_VALUE, "/f"])?;
        if !ok {
            return Err(PlatformError::Autostart);
        }
    }
    Ok(())
}

/// Whether Hece starts at login.
pub fn autostart_enabled() -> Result<bool, PlatformError> {
    reg(&["query", RUN_KEY, "/v", RUN_VALUE])
}

/// Replaces `path`'s ACL with one full-control entry for the current user (no inherited
/// entries), via `icacls.exe`.
pub fn restrict_to_user(path: &Path) -> Result<(), PlatformError> {
    let (Some(domain), Some(user)) = (std::env::var_os("USERDOMAIN"), std::env::var_os("USERNAME"))
    else {
        return Err(PlatformError::Permissions);
    };
    let mut grant = domain;
    grant.push("\\");
    grant.push(user);
    grant.push(":F");
    let ok = Command::new("icacls.exe")
        .arg(path)
        .args(["/inheritance:r", "/grant:r"])
        .arg(grant)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .is_ok_and(|o| o.status.success());
    if ok {
        Ok(())
    } else {
        Err(PlatformError::Permissions)
    }
}

/// Opens `path` with its default application (a folder in Explorer, a file in its editor).
pub fn open(path: &Path) -> Result<(), PlatformError> {
    Command::new("explorer.exe")
        .arg(path)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map(drop)
        .map_err(|_| PlatformError::Open)
}
