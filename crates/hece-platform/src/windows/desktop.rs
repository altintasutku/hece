//! The desktop thread: global shortcuts via `RegisterHotKey` (through `global-hotkey`, no
//! low-level keyboard hook) and the tray icon, sharing one message loop. The tray state is
//! changed by posting a thread message, because the icon may only be touched on this thread.
#![allow(unsafe_code, reason = "Win32 message loop FFI wrapper")]

use std::sync::mpsc;
use std::thread::JoinHandle;

use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, MSG, PostThreadMessageW, TranslateMessage, WM_APP, WM_QUIT,
};

use crate::{DesktopEvent, HotkeyEvent, MenuAction, PlatformError, Shortcut, TrayState};

/// The record shortcut: not a Windows default and free next to apps that take Ctrl+Alt+Space
/// (Claude desktop) or Win+Alt+Space (`PowerToys`); one hand can hold it.
pub const RECORD_SHORTCUT: &str = "Ctrl+Shift+Space";
/// The cancel shortcut.
pub const CANCEL_SHORTCUT: &str = "Ctrl+Shift+Backspace";

/// Thread message carrying a [`TrayState`] in `wParam`.
const WM_TRAY_STATE: u32 = WM_APP + 1;
const ICON_SIZE: u32 = 32;

const MENU: [(MenuAction, &str); 6] = [
    (MenuAction::Toggle, "Start / stop recording"),
    (MenuAction::Repaste, "Paste last dictation"),
    (MenuAction::RepasteRaw, "Paste last raw transcript"),
    (MenuAction::Settings, "Settings…"),
    (MenuAction::Logs, "Open log folder"),
    (MenuAction::Quit, "Quit Hece"),
];

/// Hotkeys and the tray icon; dropping this removes both.
#[derive(Debug)]
pub struct Desktop {
    thread_id: u32,
    thread: Option<JoinHandle<()>>,
}

impl Desktop {
    /// Registers the shortcuts, shows the tray icon and calls `handler` for every shortcut
    /// press/release and menu click. The handler runs on the desktop thread and must be quick
    /// (e.g. send on a channel).
    pub fn start(
        handler: impl Fn(DesktopEvent) + Send + Sync + 'static,
    ) -> Result<Self, PlatformError> {
        let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<u32, PlatformError>>(1);
        let thread = std::thread::Builder::new()
            .name("hece-desktop".to_owned())
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

    /// Shows `state` in the tray icon and tooltip.
    pub fn set_state(&self, state: TrayState) {
        // SAFETY: posting a message to a thread id has no memory-safety preconditions; if the
        // thread is gone the call just fails.
        unsafe { PostThreadMessageW(self.thread_id, WM_TRAY_STATE, state as usize, 0) };
    }
}

fn state_from(value: usize) -> TrayState {
    [
        TrayState::Idle,
        TrayState::Recording,
        TrayState::Processing,
        TrayState::Warning,
        TrayState::Error,
    ]
    .into_iter()
    .find(|s| *s as usize == value)
    .unwrap_or(TrayState::Idle)
}

fn tooltip(state: TrayState) -> String {
    let what = match state {
        TrayState::Idle => format!("ready ({RECORD_SHORTCUT})"),
        TrayState::Recording => "recording".to_owned(),
        TrayState::Processing => "transcribing".to_owned(),
        TrayState::Warning => "may be misheard".to_owned(),
        TrayState::Error => "something failed; see the notification".to_owned(),
    };
    format!("Hece: {what}")
}

/// A filled (or, for errors, ring-shaped) circle in the state's colour.
fn icon(state: TrayState) -> Option<Icon> {
    let (rgb, ring) = match state {
        TrayState::Idle => ([0x6B, 0x72, 0x80], false),
        TrayState::Recording => ([0xE5, 0x48, 0x4D], false),
        TrayState::Processing => ([0x3B, 0x82, 0xF6], false),
        TrayState::Warning => ([0xF5, 0xA5, 0x24], false),
        TrayState::Error => ([0xE5, 0x48, 0x4D], true),
    };
    let size = ICON_SIZE;
    let center = f64::from(size) / 2.0 - 0.5;
    let radius = f64::from(size) / 2.0 - 2.0;
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let d = (f64::from(x) - center).hypot(f64::from(y) - center);
            let inside = if ring {
                d <= radius && d >= radius - 4.0
            } else {
                d <= radius
            };
            rgba.extend_from_slice(&rgb);
            rgba.push(if inside { 0xFF } else { 0x00 });
        }
    }
    Icon::from_rgba(rgba, size, size).ok()
}

fn tray() -> Option<TrayIcon> {
    let menu = Menu::new();
    for (i, (action, label)) in MENU.iter().enumerate() {
        if *action == MenuAction::Settings || *action == MenuAction::Quit {
            menu.append(&PredefinedMenuItem::separator()).ok()?;
        }
        menu.append(&MenuItem::with_id(i.to_string(), label, true, None))
            .ok()?;
    }
    let mut builder = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip(tooltip(TrayState::Idle));
    if let Some(icon) = icon(TrayState::Idle) {
        builder = builder.with_icon(icon);
    }
    builder.build().ok()
}

fn run(
    handler: impl Fn(DesktopEvent) + Send + Sync + 'static,
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
    // A missing tray (e.g. no shell yet) is not fatal; shortcuts still work.
    let tray = tray();
    if tray.is_none() {
        tracing::warn!("tray icon could not be created");
    }
    let handler = std::sync::Arc::new(handler);
    let (record_id, cancel_id) = (record.id(), cancel.id());
    let hotkey_handler = std::sync::Arc::clone(&handler);
    GlobalHotKeyEvent::set_event_handler(Some(move |e: GlobalHotKeyEvent| {
        let shortcut = match e.id() {
            id if id == record_id => Shortcut::Record,
            id if id == cancel_id => Shortcut::Cancel,
            _ => return,
        };
        hotkey_handler(DesktopEvent::Hotkey(match e.state() {
            HotKeyState::Pressed => HotkeyEvent::Pressed(shortcut),
            HotKeyState::Released => HotkeyEvent::Released(shortcut),
        }));
    }));
    MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
        if let Some((action, _)) = e
            .id()
            .as_ref()
            .parse::<usize>()
            .ok()
            .and_then(|i| MENU.get(i))
        {
            handler(DesktopEvent::Menu(*action));
        }
    }));
    // SAFETY: GetCurrentThreadId has no preconditions.
    let thread_id = unsafe { GetCurrentThreadId() };
    let _ = ready.send(Ok(thread_id));
    // SAFETY: MSG is a plain C struct for which all-zero bytes are a valid value.
    let mut msg: MSG = unsafe { std::mem::zeroed() };
    // SAFETY: `msg` is a valid, writable MSG; a null window handle reads all of this thread's
    // messages. GetMessageW returns 0 on WM_QUIT and -1 on error, both end the loop.
    while unsafe { GetMessageW(&raw mut msg, std::ptr::null_mut(), 0, 0) } > 0 {
        if msg.hwnd.is_null() && msg.message == WM_TRAY_STATE {
            if let Some(tray) = &tray {
                let state = state_from(msg.wParam);
                let _ = tray.set_icon(icon(state));
                let _ = tray.set_tooltip(Some(tooltip(state)));
            }
            continue;
        }
        // SAFETY: `msg` was just filled in by GetMessageW.
        unsafe {
            TranslateMessage(&raw const msg);
            DispatchMessageW(&raw const msg);
        }
    }
    GlobalHotKeyEvent::set_event_handler(None::<fn(GlobalHotKeyEvent)>);
    MenuEvent::set_event_handler(None::<fn(MenuEvent)>);
    drop(tray);
    drop(manager);
}

impl Drop for Desktop {
    fn drop(&mut self) {
        // SAFETY: posting WM_QUIT to a thread id has no memory-safety preconditions; if the
        // thread already exited the call just fails.
        unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, 0, 0) };
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
