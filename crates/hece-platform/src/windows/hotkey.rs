//! Global shortcuts via `RegisterHotKey` (through `global-hotkey`) on a dedicated thread that
//! runs the message loop the hotkey window needs. No low-level keyboard hook is installed.
#![allow(unsafe_code, reason = "Win32 message loop FFI wrapper")]

use std::sync::mpsc;
use std::thread::JoinHandle;

use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, MSG, PostThreadMessageW, TranslateMessage, WM_QUIT,
};

use crate::{HotkeyEvent, PlatformError, Shortcut};

/// The record shortcut: not a Windows default and free next to apps that take Ctrl+Alt+Space
/// (Claude desktop) or Win+Alt+Space (`PowerToys`); one hand can hold it.
pub const RECORD_SHORTCUT: &str = "Ctrl+Shift+Space";
/// The cancel shortcut.
pub const CANCEL_SHORTCUT: &str = "Ctrl+Shift+Backspace";

/// Registered global shortcuts; dropping this unregisters them.
#[derive(Debug)]
pub struct Hotkeys {
    thread_id: u32,
    thread: Option<JoinHandle<()>>,
}

impl Hotkeys {
    /// Registers the shortcuts and calls `handler` for every press and release. The handler
    /// runs on the hotkey thread and must be quick (e.g. send on a channel).
    pub fn listen(
        handler: impl Fn(HotkeyEvent) + Send + Sync + 'static,
    ) -> Result<Self, PlatformError> {
        let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<u32, PlatformError>>(1);
        let thread = std::thread::Builder::new()
            .name("hece-hotkeys".to_owned())
            .spawn(move || run(handler, &ready_tx))
            .map_err(|_| PlatformError::Hotkey)?;
        match ready_rx.recv() {
            Ok(Ok(thread_id)) => Ok(Self {
                thread_id,
                thread: Some(thread),
            }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => {
                let _ = thread.join();
                Err(PlatformError::Hotkey)
            }
        }
    }
}

fn run(
    handler: impl Fn(HotkeyEvent) + Send + Sync + 'static,
    ready: &mpsc::SyncSender<Result<u32, PlatformError>>,
) {
    let mods = Some(Modifiers::CONTROL | Modifiers::SHIFT);
    let record = HotKey::new(mods, Code::Space);
    let cancel = HotKey::new(mods, Code::Backspace);
    let Ok(manager) = GlobalHotKeyManager::new() else {
        let _ = ready.send(Err(PlatformError::Hotkey));
        return;
    };
    for (key, name) in [(record, RECORD_SHORTCUT), (cancel, CANCEL_SHORTCUT)] {
        if let Err(e) = manager.register(key) {
            tracing::warn!(shortcut = name, error = %e, "hotkey registration failed");
            let _ = ready.send(Err(PlatformError::HotkeyTaken(name)));
            return;
        }
    }
    let (record_id, cancel_id) = (record.id(), cancel.id());
    GlobalHotKeyEvent::set_event_handler(Some(move |e: GlobalHotKeyEvent| {
        let shortcut = match e.id() {
            id if id == record_id => Shortcut::Record,
            id if id == cancel_id => Shortcut::Cancel,
            _ => return,
        };
        handler(match e.state() {
            HotKeyState::Pressed => HotkeyEvent::Pressed(shortcut),
            HotKeyState::Released => HotkeyEvent::Released(shortcut),
        });
    }));
    // SAFETY: GetCurrentThreadId has no preconditions.
    let thread_id = unsafe { GetCurrentThreadId() };
    let _ = ready.send(Ok(thread_id));
    // SAFETY: MSG is a plain C struct for which all-zero bytes are a valid value.
    let mut msg: MSG = unsafe { std::mem::zeroed() };
    // SAFETY: `msg` is a valid, writable MSG; a null window handle reads all of this thread's
    // messages. GetMessageW returns 0 on WM_QUIT and -1 on error, both end the loop.
    while unsafe { GetMessageW(&raw mut msg, std::ptr::null_mut(), 0, 0) } > 0 {
        // SAFETY: `msg` was just filled in by GetMessageW.
        unsafe {
            TranslateMessage(&raw const msg);
            DispatchMessageW(&raw const msg);
        }
    }
    GlobalHotKeyEvent::set_event_handler(None::<fn(GlobalHotKeyEvent)>);
    drop(manager);
}

impl Drop for Hotkeys {
    fn drop(&mut self) {
        // SAFETY: posting WM_QUIT to a thread id has no memory-safety preconditions; if the
        // thread already exited the call just fails.
        unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, 0, 0) };
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
