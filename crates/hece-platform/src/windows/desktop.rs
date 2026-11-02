//! The desktop thread: global shortcuts via `RegisterHotKey` (through `global-hotkey`, no
//! low-level keyboard hook) and the tray icon, sharing one message loop. The tray state is
//! changed by posting a thread message, because the icon may only be touched on this thread.
#![allow(unsafe_code, reason = "Win32 message loop FFI wrapper")]

use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use hece_core::hotkey::Hotkey;
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, MSG, PostThreadMessageW, TranslateMessage, WM_APP, WM_QUIT,
};

use crate::{
    DesktopEvent, HotkeyEvent, MenuAction, PlatformError, Shortcut, TrayState, menu_label,
    tray_status, ui_language,
};

/// Thread message carrying a [`TrayState`] in `wParam`.
const WM_TRAY_STATE: u32 = WM_APP + 1;
/// Thread message: a [`Rebind`] is waiting on the channel.
const WM_REBIND: u32 = WM_APP + 2;
/// How long [`Desktop::set_hotkeys`] waits for the desktop thread.
const REBIND_TIMEOUT: Duration = Duration::from_secs(2);
const ICON_SIZE: u32 = 32;

/// New shortcuts for the desktop thread, with where to send the outcome.
#[derive(Debug)]
struct Rebind {
    record: Hotkey,
    cancel: Hotkey,
    reply: mpsc::SyncSender<Result<(), PlatformError>>,
}

const MENU: [MenuAction; 6] = [
    MenuAction::Toggle,
    MenuAction::Repaste,
    MenuAction::RepasteRaw,
    MenuAction::Settings,
    MenuAction::Logs,
    MenuAction::Quit,
];

/// Hotkeys and the tray icon; dropping this removes both.
#[derive(Debug)]
pub struct Desktop {
    thread_id: u32,
    thread: Option<JoinHandle<()>>,
    rebind: mpsc::Sender<Rebind>,
}

impl Desktop {
    /// Registers the `record` and `cancel` shortcuts, shows the tray icon and calls `handler`
    /// for every shortcut press/release and menu click. The handler runs on the desktop thread
    /// and must be quick (e.g. send on a channel).
    pub fn start(
        record: Hotkey,
        cancel: Hotkey,
        handler: impl Fn(DesktopEvent) + Send + Sync + 'static,
    ) -> Result<Self, PlatformError> {
        let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<u32, PlatformError>>(1);
        let (rebind, rebind_rx) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("hece-desktop".to_owned())
            .spawn(move || run(Bound::new(record, cancel), handler, &ready_tx, &rebind_rx))
            .map_err(|_| PlatformError::Hotkey)?;
        match ready_rx.recv() {
            Ok(Ok(thread_id)) => Ok(Self {
                thread_id,
                thread: Some(thread),
                rebind,
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

    /// Replaces both shortcuts. If one cannot be registered (another application owns it),
    /// the previous pair stays active and [`PlatformError::HotkeyTaken`] names the culprit.
    pub fn set_hotkeys(&self, record: Hotkey, cancel: Hotkey) -> Result<(), PlatformError> {
        let (reply, answer) = mpsc::sync_channel(1);
        self.rebind
            .send(Rebind {
                record,
                cancel,
                reply,
            })
            .map_err(|_| PlatformError::Hotkey)?;
        // SAFETY: posting a message to a thread id has no memory-safety preconditions; if the
        // thread is gone the call just fails and the wait below times out.
        unsafe { PostThreadMessageW(self.thread_id, WM_REBIND, 0, 0) };
        answer
            .recv_timeout(REBIND_TIMEOUT)
            .map_err(|_| PlatformError::Hotkey)?
    }
}

/// The registered pair, as configured and as `global-hotkey` keys.
struct Bound {
    record: Hotkey,
    cancel: Hotkey,
}

impl Bound {
    fn new(record: Hotkey, cancel: Hotkey) -> Self {
        Self { record, cancel }
    }

    fn keys(&self) -> Result<(HotKey, HotKey), PlatformError> {
        Ok((to_hotkey(&self.record)?, to_hotkey(&self.cancel)?))
    }

    /// Registers both or neither.
    fn register(&self, manager: &GlobalHotKeyManager) -> Result<(HotKey, HotKey), PlatformError> {
        let (record, cancel) = self.keys()?;
        if let Err(e) = manager.register(record) {
            tracing::warn!(shortcut = %self.record, error = %e, "hotkey registration failed");
            return Err(PlatformError::HotkeyTaken(self.record.to_string()));
        }
        if let Err(e) = manager.register(cancel) {
            tracing::warn!(shortcut = %self.cancel, error = %e, "hotkey registration failed");
            let _ = manager.unregister(record);
            return Err(PlatformError::HotkeyTaken(self.cancel.to_string()));
        }
        Ok((record, cancel))
    }
}

/// The `RegisterHotKey` key for a configured shortcut.
fn to_hotkey(hotkey: &Hotkey) -> Result<HotKey, PlatformError> {
    let m = hotkey.modifiers;
    let mut mods = Modifiers::empty();
    for (on, flag) in [
        (m.ctrl, Modifiers::CONTROL),
        (m.alt, Modifiers::ALT),
        (m.shift, Modifiers::SHIFT),
        (m.win, Modifiers::META),
    ] {
        if on {
            mods |= flag;
        }
    }
    let code = Code::from_str(&hotkey.key.code()).map_err(|_| PlatformError::Hotkey)?;
    Ok(HotKey::new(Some(mods), code))
}

/// The ids the event handler matches: record, cancel.
type Ids = Arc<[AtomicU32; 2]>;

fn store_ids(ids: &Ids, (record, cancel): (HotKey, HotKey)) {
    ids[0].store(record.id(), Ordering::SeqCst);
    ids[1].store(cancel.id(), Ordering::SeqCst);
}

/// Swaps the registered pair for `request`'s, restoring the old pair if that fails.
fn rebind(manager: &GlobalHotKeyManager, bound: &mut Bound, ids: &Ids, request: &Rebind) {
    let next = Bound::new(request.record, request.cancel);
    if next.record == bound.record && next.cancel == bound.cancel {
        let _ = request.reply.send(Ok(()));
        return;
    }
    if let Ok((record, cancel)) = bound.keys() {
        let _ = manager.unregister(record);
        let _ = manager.unregister(cancel);
    }
    let result = match next.register(manager) {
        Ok(keys) => {
            store_ids(ids, keys);
            *bound = next;
            Ok(())
        }
        Err(e) => {
            if bound.register(manager).is_err() {
                tracing::error!("previous shortcuts could not be registered again");
            }
            Err(e)
        }
    };
    let _ = request.reply.send(result);
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

fn tooltip(state: TrayState, record: &Hotkey) -> String {
    format!(
        "Hece: {}",
        tray_status(state, ui_language(), &record.to_string())
    )
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

fn tray(record: &Hotkey) -> Option<TrayIcon> {
    let menu = Menu::new();
    for (i, action) in MENU.iter().enumerate() {
        if *action == MenuAction::Settings || *action == MenuAction::Quit {
            menu.append(&PredefinedMenuItem::separator()).ok()?;
        }
        menu.append(&MenuItem::with_id(
            i.to_string(),
            menu_label(*action, ui_language()),
            true,
            None,
        ))
        .ok()?;
    }
    let mut builder = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip(tooltip(TrayState::Idle, record));
    if let Some(icon) = icon(TrayState::Idle) {
        builder = builder.with_icon(icon);
    }
    builder.build().ok()
}

fn run(
    mut bound: Bound,
    handler: impl Fn(DesktopEvent) + Send + Sync + 'static,
    ready: &mpsc::SyncSender<Result<u32, PlatformError>>,
    rebinds: &mpsc::Receiver<Rebind>,
) {
    let Ok(manager) = GlobalHotKeyManager::new() else {
        let _ = ready.send(Err(PlatformError::Hotkey));
        return;
    };
    let ids: Ids = Arc::new([AtomicU32::new(0), AtomicU32::new(0)]);
    match bound.register(&manager) {
        Ok(keys) => store_ids(&ids, keys),
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    }
    // A missing tray (e.g. no shell yet) is not fatal; shortcuts still work.
    let tray = tray(&bound.record);
    if tray.is_none() {
        tracing::warn!("tray icon could not be created");
    }
    let mut state = TrayState::Idle;
    let handler = Arc::new(handler);
    let hotkey_handler = Arc::clone(&handler);
    let handler_ids = Arc::clone(&ids);
    GlobalHotKeyEvent::set_event_handler(Some(move |e: GlobalHotKeyEvent| {
        let shortcut = match e.id() {
            id if id == handler_ids[0].load(Ordering::SeqCst) => Shortcut::Record,
            id if id == handler_ids[1].load(Ordering::SeqCst) => Shortcut::Cancel,
            _ => return,
        };
        hotkey_handler(DesktopEvent::Hotkey(match e.state() {
            HotKeyState::Pressed => HotkeyEvent::Pressed(shortcut),
            HotKeyState::Released => HotkeyEvent::Released(shortcut),
        }));
    }));
    MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
        if let Some(action) = e
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
            state = state_from(msg.wParam);
            if let Some(tray) = &tray {
                let _ = tray.set_icon(icon(state));
                let _ = tray.set_tooltip(Some(tooltip(state, &bound.record)));
            }
            continue;
        }
        if msg.hwnd.is_null() && msg.message == WM_REBIND {
            while let Ok(request) = rebinds.try_recv() {
                rebind(&manager, &mut bound, &ids, &request);
            }
            if let Some(tray) = &tray {
                let _ = tray.set_tooltip(Some(tooltip(state, &bound.record)));
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
