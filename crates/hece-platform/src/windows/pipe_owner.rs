//! Who is on the other end of a named pipe. Pipe names are predictable and machine-wide, so
//! another account could create ours first; clients check the server before sending anything.
#![allow(unsafe_code, reason = "Win32 process token FFI wrapper")]

use std::os::windows::io::AsRawHandle;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::Security::{
    EqualSid, GetTokenInformation, TOKEN_QUERY, TOKEN_USER, TokenUser,
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

/// The `TOKEN_USER` of `process`, in a buffer aligned for it.
fn token_user(process: HANDLE) -> Option<Vec<u64>> {
    let mut token: HANDLE = std::ptr::null_mut();
    // SAFETY: `token` is a valid out pointer; a failure leaves it null.
    if unsafe { OpenProcessToken(process, TOKEN_QUERY, &raw mut token) } == 0 {
        return None;
    }
    let token = Owned(token);
    let mut len = 0u32;
    // SAFETY: a null buffer of length 0 only asks for the required length.
    unsafe { GetTokenInformation(token.0, TokenUser, std::ptr::null_mut(), 0, &raw mut len) };
    let mut buf = vec![0u64; (len as usize).div_ceil(8)];
    // SAFETY: `buf` holds at least `len` bytes and is 8-byte aligned, enough for TOKEN_USER.
    let ok = unsafe {
        GetTokenInformation(
            token.0,
            TokenUser,
            buf.as_mut_ptr().cast(),
            len,
            &raw mut len,
        )
    };
    (ok != 0).then_some(buf)
}

/// Whether the server end of `pipe` runs as the current user. Anything that cannot be
/// checked (for example another account's process we may not open) counts as foreign.
#[must_use]
pub fn pipe_server_is_current_user(pipe: &impl AsRawHandle) -> bool {
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
    let (Some(theirs), Some(ours)) = (
        token_user(server.0),
        token_user(unsafe { GetCurrentProcess() }),
    ) else {
        return false;
    };
    let (theirs, ours) = (
        theirs.as_ptr().cast::<TOKEN_USER>(),
        ours.as_ptr().cast::<TOKEN_USER>(),
    );
    // SAFETY: both buffers hold a TOKEN_USER filled by GetTokenInformation and outlive the call;
    // their SID pointers point into the same buffers.
    unsafe { EqualSid((*theirs).User.Sid, (*ours).User.Sid) != 0 }
}

#[cfg(test)]
mod tests {
    use tokio::net::windows::named_pipe::{ClientOptions, ServerOptions};

    #[tokio::test]
    async fn own_pipe_is_current_user() {
        let name = format!(r"\\.\pipe\hece-test-owner-{}", std::process::id());
        let _server = ServerOptions::new().create(&name).unwrap();
        let client = ClientOptions::new().open(&name).unwrap();
        assert!(super::pipe_server_is_current_user(&client));
    }
}
