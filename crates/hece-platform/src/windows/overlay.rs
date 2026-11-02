//! The recording overlay window: a layered, click-through, never-activated popup in the top-right
//! corner of the monitor the user types on. It runs on its own thread (keeping the frame timer off
//! the hotkey thread) and pushes each frame of [`Scene`] with `UpdateLayeredWindow`. While hidden
//! it has no timer and draws nothing.
#![allow(unsafe_code, reason = "Win32 layered window FFI wrapper")]

use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Instant;

use tiny_skia::Pixmap;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION,
    CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetMonitorInfoW,
    HBITMAP, HDC, HGDIOBJ, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromWindow, SelectObject,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForMonitor, MDT_EFFECTIVE_DPI,
    SetThreadDpiAwarenessContext,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetForegroundWindow,
    GetMessageW, HTTRANSPARENT, HWND_TOPMOST, KillTimer, MA_NOACTIVATE, MSG, PostThreadMessageW,
    RegisterClassExW, SPI_GETCLIENTAREAANIMATION, SW_HIDE, SW_SHOWNOACTIVATE, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOSIZE, SetTimer, SetWindowPos, ShowWindow, SystemParametersInfoW, ULW_ALPHA,
    UpdateLayeredWindow, WM_APP, WM_MOUSEACTIVATE, WM_NCHITTEST, WM_QUIT, WM_TIMER, WNDCLASSEXW,
    WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

use crate::PlatformError;
use crate::overlay::{Phase, Scene};

/// Thread message carrying a [`Phase`] (see [`Phase::to_raw`]) in `wParam`.
const WM_OVERLAY_PHASE: u32 = WM_APP + 1;
/// Logical size of the overlay, in pixels at 100 % scale.
const SIZE_PX: u32 = 140;
/// Logical distance from the work area's top and right edges.
const MARGIN_PX: u32 = 16;
/// Frame interval; Windows rounds timers to its tick, so this is about 60 fps.
const FRAME_MS: u32 = 16;
const TIMER_ID: usize = 1;

/// The overlay window and its thread; dropping this closes both.
#[derive(Debug)]
pub struct Overlay {
    thread_id: u32,
    thread: Option<JoinHandle<()>>,
}

impl Overlay {
    /// Creates the hidden overlay window on its own thread. `level` returns the latest capture
    /// RMS; it is called once per frame while the overlay shows.
    pub fn start(level: impl Fn() -> f32 + Send + 'static) -> Result<Self, PlatformError> {
        let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<u32, PlatformError>>(1);
        let thread = std::thread::Builder::new()
            .name("hece-overlay".to_owned())
            .spawn(move || run(&level, &ready_tx))
            .map_err(|_| PlatformError::Overlay)?;
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
                Err(PlatformError::Overlay)
            }
        }
    }

    /// Shows `phase`. Setting the current phase again changes nothing.
    pub fn set_phase(&self, phase: Phase) {
        // SAFETY: posting a message to a thread id has no memory-safety preconditions; if the
        // thread is gone the call just fails.
        unsafe { PostThreadMessageW(self.thread_id, WM_OVERLAY_PHASE, phase.to_raw(), 0) };
    }
}

impl Drop for Overlay {
    fn drop(&mut self) {
        // SAFETY: as in `set_phase`.
        unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, 0, 0) };
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Never hit-tested and never activated, so clicks reach the window below and focus stays put.
unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_NCHITTEST => HTTRANSPARENT as LRESULT,
        WM_MOUSEACTIVATE => LRESULT::try_from(MA_NOACTIVATE).unwrap_or(3),
        // SAFETY: forwarding the arguments Windows passed to this window procedure.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn create_window() -> Option<HWND> {
    let class = wide("HeceOverlay");
    // SAFETY: a null module name returns this executable's handle; no other preconditions.
    let instance = unsafe { GetModuleHandleW(std::ptr::null()) };
    let wc = WNDCLASSEXW {
        cbSize: u32::try_from(size_of::<WNDCLASSEXW>()).ok()?,
        lpfnWndProc: Some(window_proc),
        hInstance: instance,
        lpszClassName: class.as_ptr(),
        // SAFETY: WNDCLASSEXW is a plain C struct; the remaining all-zero fields are valid
        // "none" values.
        ..unsafe { std::mem::zeroed() }
    };
    // Fails harmlessly when the class exists from an earlier overlay in this process.
    // SAFETY: `wc` is fully initialised and `class` outlives the call.
    unsafe { RegisterClassExW(&raw const wc) };
    let title = wide("Hece");
    // SAFETY: the class was registered above (or earlier); all pointers are valid for the call.
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
            class.as_ptr(),
            title.as_ptr(),
            WS_POPUP,
            0,
            0,
            1,
            1,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        )
    };
    (!hwnd.is_null()).then_some(hwnd)
}

/// Whether Windows "Animation effects" are off.
fn reduced_motion() -> bool {
    let mut enabled: i32 = 1;
    // SAFETY: SPI_GETCLIENTAREAANIMATION writes one BOOL to the pointer, which is valid.
    let ok = unsafe {
        SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, (&raw mut enabled).cast(), 0)
    };
    ok != 0 && enabled == 0
}

/// Top-left corner and side length, in physical pixels, on the monitor of the foreground
/// window (the one the user is typing in).
fn placement() -> (POINT, u32) {
    // SAFETY: no preconditions; a null result makes MonitorFromWindow fall back to the primary.
    let monitor = unsafe { MonitorFromWindow(GetForegroundWindow(), MONITOR_DEFAULTTOPRIMARY) };
    let mut info = MONITORINFO {
        cbSize: u32::try_from(size_of::<MONITORINFO>()).unwrap_or(0),
        rcMonitor: RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        },
        rcWork: RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        },
        dwFlags: 0,
    };
    // SAFETY: `info` is a valid MONITORINFO with cbSize set.
    unsafe { GetMonitorInfoW(monitor, &raw mut info) };
    let (mut dpi_x, mut dpi_y) = (96_u32, 96_u32);
    // SAFETY: both out pointers are valid; on failure they keep 96 (100 %).
    unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &raw mut dpi_x, &raw mut dpi_y) };
    let side = SIZE_PX * dpi_x / 96;
    let margin = MARGIN_PX * dpi_x / 96;
    let side_i = i32::try_from(side).unwrap_or(140);
    let margin_i = i32::try_from(margin).unwrap_or(16);
    let corner = POINT {
        x: info.rcWork.right - margin_i - side_i,
        y: info.rcWork.top + margin_i,
    };
    (corner, side)
}

/// A memory DC holding a top-down 32-bit DIB that `UpdateLayeredWindow` reads from.
struct Surface {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    bits: *mut u8,
    side: u32,
}

impl Surface {
    fn new(side: u32) -> Option<Self> {
        let height = i32::try_from(side).ok()?;
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: u32::try_from(size_of::<BITMAPINFOHEADER>()).ok()?,
                biWidth: height,
                // Negative height: rows top to bottom, like the pixmap.
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                biSizeImage: 0,
                biXPelsPerMeter: 0,
                biYPelsPerMeter: 0,
                biClrUsed: 0,
                biClrImportant: 0,
            },
            // SAFETY: RGBQUAD is a plain C struct; zero is a valid value.
            bmiColors: [unsafe { std::mem::zeroed() }],
        };
        // SAFETY: a null DC makes a DC compatible with the screen.
        let dc = unsafe { CreateCompatibleDC(std::ptr::null_mut()) };
        if dc.is_null() {
            return None;
        }
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        // SAFETY: `info` describes a valid 32-bit DIB; `bits` receives its pixel pointer.
        let bitmap = unsafe {
            CreateDIBSection(
                dc,
                &raw const info,
                DIB_RGB_COLORS,
                &raw mut bits,
                std::ptr::null_mut(),
                0,
            )
        };
        if bitmap.is_null() || bits.is_null() {
            // SAFETY: `dc` was created above and is not selected into anything.
            unsafe { DeleteDC(dc) };
            return None;
        }
        // SAFETY: both handles are valid; the previous object is restored in Drop.
        let previous = unsafe { SelectObject(dc, bitmap) };
        Some(Self {
            dc,
            bitmap,
            previous,
            bits: bits.cast(),
            side,
        })
    }

    /// Copies a premultiplied RGBA pixmap of the same size as premultiplied BGRA.
    fn copy_from(&mut self, pixmap: &Pixmap) {
        let len = (self.side * self.side * 4) as usize;
        let source = pixmap.data();
        if source.len() != len {
            return;
        }
        // SAFETY: the DIB section is `side * side` 32-bit pixels, owned by this Surface and
        // only touched on this thread.
        let target = unsafe { std::slice::from_raw_parts_mut(self.bits, len) };
        for (dst, src) in target.chunks_exact_mut(4).zip(source.chunks_exact(4)) {
            dst.copy_from_slice(&[src[2], src[1], src[0], src[3]]);
        }
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        // SAFETY: restores the DC's original bitmap before deleting the objects created in `new`.
        unsafe {
            SelectObject(self.dc, self.previous);
            DeleteObject(self.bitmap);
            DeleteDC(self.dc);
        }
    }
}

/// The window's frame state on the overlay thread.
struct Frames {
    hwnd: HWND,
    scene: Scene,
    corner: POINT,
    pixmap: Option<Pixmap>,
    surface: Option<Surface>,
}

impl Frames {
    /// Draws the scene and pushes it to the window.
    fn present(&mut self) {
        let Some(pixmap) = self.pixmap.as_mut() else {
            return;
        };
        let Some(surface) = self.surface.as_mut() else {
            return;
        };
        self.scene.draw(pixmap);
        surface.copy_from(pixmap);
        let edge = i32::try_from(surface.side).unwrap_or(0);
        let size = SIZE { cx: edge, cy: edge };
        let origin = POINT { x: 0, y: 0 };
        let blend = BLENDFUNCTION {
            BlendOp: u8::try_from(AC_SRC_OVER).unwrap_or(0),
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: u8::try_from(AC_SRC_ALPHA).unwrap_or(1),
        };
        // SAFETY: the window and the surface's DC are valid; all pointers point to locals.
        unsafe {
            UpdateLayeredWindow(
                self.hwnd,
                std::ptr::null_mut(),
                &raw const self.corner,
                &raw const size,
                surface.dc,
                &raw const origin,
                0,
                &raw const blend,
                ULW_ALPHA,
            );
        }
    }

    /// Places, sizes and shows the window without activating it, then starts the frame timer.
    fn show(&mut self) {
        let (corner, side) = placement();
        self.corner = corner;
        if self.surface.as_ref().is_none_or(|s| s.side != side) {
            self.surface = Surface::new(side);
            self.pixmap = Pixmap::new(side, side);
        }
        // The first frame goes in before the window shows, so it never flashes empty.
        self.present();
        // SAFETY: `hwnd` is this thread's window; none of these calls activate it.
        unsafe {
            ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
            SetWindowPos(
                self.hwnd,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
            SetTimer(self.hwnd, TIMER_ID, FRAME_MS, None);
        }
    }

    fn hide(&self) {
        // SAFETY: `hwnd` is this thread's window.
        unsafe {
            KillTimer(self.hwnd, TIMER_ID);
            ShowWindow(self.hwnd, SW_HIDE);
        }
    }
}

fn run(level: &impl Fn() -> f32, ready: &mpsc::SyncSender<Result<u32, PlatformError>>) {
    // Physical pixels everywhere on this thread, whatever the daemon's DPI mode.
    // SAFETY: no preconditions; on old Windows it fails and the window is just scaled.
    unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    let Some(hwnd) = create_window() else {
        tracing::warn!("overlay window could not be created");
        let _ = ready.send(Err(PlatformError::Overlay));
        return;
    };
    // SAFETY: GetCurrentThreadId has no preconditions.
    let thread_id = unsafe { GetCurrentThreadId() };
    let _ = ready.send(Ok(thread_id));
    let started = Instant::now();
    let mut frames = Frames {
        hwnd,
        scene: Scene::new(false),
        corner: POINT { x: 0, y: 0 },
        pixmap: None,
        surface: None,
    };
    // SAFETY: MSG is a plain C struct for which all-zero bytes are a valid value.
    let mut msg: MSG = unsafe { std::mem::zeroed() };
    // SAFETY: `msg` is a valid, writable MSG; a null window handle reads all of this thread's
    // messages. GetMessageW returns 0 on WM_QUIT and -1 on error, both end the loop.
    while unsafe { GetMessageW(&raw mut msg, std::ptr::null_mut(), 0, 0) } > 0 {
        let now = started.elapsed();
        if msg.hwnd.is_null() && msg.message == WM_OVERLAY_PHASE {
            let phase = Phase::from_raw(msg.wParam);
            if frames.scene.visible() {
                frames.scene.set_phase(phase, now);
            } else {
                let mut scene = Scene::new(reduced_motion());
                scene.set_phase(phase, now);
                if scene.visible() {
                    frames.scene = scene;
                    frames.scene.tick(now, level());
                    frames.show();
                }
            }
            continue;
        }
        if msg.message == WM_TIMER && msg.hwnd == hwnd {
            if frames.scene.tick(now, level()) {
                frames.present();
            } else {
                frames.hide();
            }
            continue;
        }
        // SAFETY: `msg` was just filled in by GetMessageW.
        unsafe { DispatchMessageW(&raw const msg) };
    }
    drop(frames.surface.take());
    // SAFETY: the window belongs to this thread and is destroyed once, on the way out.
    unsafe { DestroyWindow(hwnd) };
}
