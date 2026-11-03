//! Who is on the other end of a named pipe. Pipe names live in one machine-wide namespace, so
//! another process could create ours first; clients check the server before sending anything.
#![allow(unsafe_code, reason = "Win32 process token FFI wrapper")]

use std::os::windows::io::AsRawHandle;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::Security::{
    EqualSid, GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation,
    TOKEN_INFORMATION_CLASS, TOKEN_MANDATORY_LABEL, TOKEN_QUERY, TOKEN_USER, TokenIntegrityLevel,
    TokenIsAppContainer, TokenUser,
};
use windows_sys::Win32::System::Pipes::GetNamedPipeServerProcessId;
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
};

/// Closes a handle when dropped.
struct Owned(HANDLE);

impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: the handle came from a successful Open* call and is closed exactly once.
        unsafe { CloseHandle(self.0) };
    }
}

/// What the check needs from a process token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Trust {
    /// The process runs in an `AppContainer` (a sandboxed app).
    app_container: bool,
    /// Mandatory integrity level RID (`SECURITY_MANDATORY_*_RID`).
    integrity: u32,
}

/// Whether a server of the same user with `server` trust may serve a client with `ours`:
/// not sandboxed and not of lower integrity, so a low-integrity or `AppContainer` process of the
/// same user that creates the pipe first is not trusted.
fn trusted(same_user: bool, server: Trust, ours: Trust) -> bool {
    same_user && !server.app_container && server.integrity >= ours.integrity
}

/// One information class of `token`, in a buffer aligned for any of the token structs.
fn token_info(token: HANDLE, class: TOKEN_INFORMATION_CLASS) -> Option<Vec<u64>> {
    let mut len = 0u32;
    // SAFETY: a null buffer of length 0 only asks for the required length.
    unsafe { GetTokenInformation(token, class, std::ptr::null_mut(), 0, &raw mut len) };
    if len == 0 {
        return None;
    }
    let mut buf = vec![0u64; (len as usize).div_ceil(8)];
    // SAFETY: `buf` holds at least `len` bytes and is 8-byte aligned, enough for every
    // TOKEN_* struct.
    let ok =
        unsafe { GetTokenInformation(token, class, buf.as_mut_ptr().cast(), len, &raw mut len) };
    (ok != 0).then_some(buf)
}

/// The `TOKEN_USER` and trust of `process`'s token.
fn token_facts(process: HANDLE) -> Option<(Vec<u64>, Trust)> {
    let mut token: HANDLE = std::ptr::null_mut();
    // SAFETY: `token` is a valid out pointer; a failure leaves it null.
    if unsafe { OpenProcessToken(process, TOKEN_QUERY, &raw mut token) } == 0 {
        return None;
    }
    let token = Owned(token);
    let user = token_info(token.0, TokenUser)?;
    let app_container = token_info(token.0, TokenIsAppContainer)?;
    // TokenIsAppContainer is a DWORD: the low four bytes of the zeroed, little-endian buffer.
    let app_container = app_container.first().is_some_and(|v| v & 0xFFFF_FFFF != 0);
    let label = token_info(token.0, TokenIntegrityLevel)?;
    let label = label.as_ptr().cast::<TOKEN_MANDATORY_LABEL>();
    // SAFETY: `label` holds a TOKEN_MANDATORY_LABEL filled by GetTokenInformation whose SID
    // points into the same live buffer; the subauthority count of a valid SID is readable, and
    // index `count - 1` is in range because it is checked to be at least 1.
    let integrity = unsafe {
        let sid = (*label).Label.Sid;
        let count = *GetSidSubAuthorityCount(sid);
        if count == 0 {
            return None;
        }
        *GetSidSubAuthority(sid, u32::from(count) - 1)
    };
    Some((
        user,
        Trust {
            app_container,
            integrity,
        },
    ))
}

/// Whether the server end of `pipe` may be trusted: same user as this process, not an
/// `AppContainer`, and integrity level at least ours. Anything that cannot be checked (for
/// example another account's process we may not open) counts as untrusted.
#[must_use]
pub fn pipe_server_is_trusted(pipe: &impl AsRawHandle) -> bool {
    let mut pid = 0u32;
    // SAFETY: the handle is a live pipe client handle borrowed for this call.
    if unsafe { GetNamedPipeServerProcessId(pipe.as_raw_handle(), &raw mut pid) } == 0 {
        return false;
    }
    // SAFETY: OpenProcess has no pointer arguments; a failure returns null.
    let server = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if server.is_null() {
        return false;
    }
    let server = Owned(server);
    // SAFETY: the pseudo handle of the current process needs no closing.
    let (Some((their_user, theirs)), Some((our_user, ours))) = (
        token_facts(server.0),
        token_facts(unsafe { GetCurrentProcess() }),
    ) else {
        return false;
    };
    let (their_user, our_user) = (
        their_user.as_ptr().cast::<TOKEN_USER>(),
        our_user.as_ptr().cast::<TOKEN_USER>(),
    );
    // SAFETY: both buffers hold a TOKEN_USER filled by GetTokenInformation and outlive the call;
    // their SID pointers point into the same buffers.
    let same_user = unsafe { EqualSid((*their_user).User.Sid, (*our_user).User.Sid) != 0 };
    trusted(same_user, theirs, ours)
}

#[cfg(test)]
mod tests {
    use tokio::net::windows::named_pipe::{ClientOptions, ServerOptions};

    use super::{Trust, trusted};

    #[test]
    fn trust_needs_same_user_no_sandbox_and_no_lower_integrity() {
        let medium = Trust {
            app_container: false,
            integrity: 0x2000,
        };
        let high = Trust {
            integrity: 0x3000,
            ..medium
        };
        let low = Trust {
            integrity: 0x1000,
            ..medium
        };
        let sandboxed = Trust {
            app_container: true,
            ..medium
        };
        assert!(trusted(true, medium, medium));
        assert!(trusted(true, high, medium));
        assert!(!trusted(false, medium, medium));
        assert!(!trusted(true, low, medium));
        assert!(!trusted(true, medium, high));
        assert!(!trusted(true, sandboxed, medium));
    }

    #[tokio::test]
    async fn own_pipe_is_trusted() {
        let name = format!(r"\\.\pipe\hece-test-owner-{}", std::process::id());
        let _server = ServerOptions::new().create(&name).unwrap();
        let client = ClientOptions::new().open(&name).unwrap();
        assert!(super::pipe_server_is_trusted(&client));
    }
}
