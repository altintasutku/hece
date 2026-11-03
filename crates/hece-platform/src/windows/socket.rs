//! Exclusive TCP port binding.
#![allow(unsafe_code, reason = "Winsock setsockopt FFI wrapper")]

use std::os::windows::io::{AsRawSocket as _, AsSocket};

use windows_sys::Win32::Networking::WinSock::{
    SO_EXCLUSIVEADDRUSE, SOCKET, SOL_SOCKET, WSAGetLastError, setsockopt,
};

/// Sets `SO_EXCLUSIVEADDRUSE` so no other socket (another account's included) can bind the same
/// address and port while this one is open. Call before `bind`.
pub fn exclusive_address_use(socket: &impl AsSocket) -> std::io::Result<()> {
    let raw = socket.as_socket().as_raw_socket();
    let Ok(raw) = SOCKET::try_from(raw) else {
        return Err(std::io::Error::other("socket handle out of range"));
    };
    let on: i32 = 1;
    // SAFETY: `raw` is a live socket borrowed for this call; `optval` points to a 4-byte i32
    // that outlives the call and `optlen` is its size.
    let rc = unsafe {
        setsockopt(
            raw,
            SOL_SOCKET,
            SO_EXCLUSIVEADDRUSE,
            (&raw const on).cast::<u8>(),
            4,
        )
    };
    if rc == 0 {
        Ok(())
    } else {
        // SAFETY: WSAGetLastError takes no arguments and reads this thread's last Winsock error.
        Err(std::io::Error::from_raw_os_error(unsafe {
            WSAGetLastError()
        }))
    }
}
