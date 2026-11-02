# Recording Overlay Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** While Hece records and transcribes, show a click-through, never-focused animated sphere (look "A3") in the top-right corner, driven by the live mic level and the daemon state.

**Architecture:** `hece-audio` publishes the chunk RMS through a shared `Level` atomic. A pure, portable `overlay` module in `hece-platform` turns (phase, level, time) into a tiny-skia pixmap. A Windows-only layered window on its own thread pushes that pixmap with `UpdateLayeredWindow` at ~60 fps while visible. The daemon derives the phase at the same point it refreshes the tray.

**Tech Stack:** Rust 2024, `windows-sys` 0.59, `tiny-skia` 0.12 (new, approved by Utku 2026-11-02), Svelte 5 for the settings toggle.

**Spec:** `docs/superpowers/specs/2026-11-02-recording-overlay-design.md` (+ prototype `2026-11-02-recording-overlay-prototype.html`, card A3, functions `project`, `drawA3`, `getLevel`, `stepW`).

## Global Constraints

- Project rules in `CLAUDE.md`: no `unwrap`/`expect` outside tests; `unsafe` only in FFI modules with `#![allow(unsafe_code, reason = "...")]` and a `// SAFETY:` comment per block; every `#[allow]` has `reason`; no debug prints; `thiserror` per crate.
- The overlay window is never activated: `WS_EX_NOACTIVATE`, `MA_NOACTIVATE`, shown only with `SW_SHOWNOACTIVATE`.
- Click-through (`WS_EX_TRANSPARENT`, `HTTRANSPARENT`), not in taskbar or Alt+Tab (`WS_EX_TOOLWINDOW`), topmost.
- Zero idle cost: no timer while hidden.
- Size 140×140 logical px, 16 logical px margin from the work area's top-right, monitor of the foreground window, per-monitor DPI aware V2 on the overlay thread.
- Only a scalar level reaches the overlay; levels and audio are never logged.
- Exit animations last 300 ms.
- Level mapping: dBFS −50..−15 → 0..1; attack 0.35, release 0.08 per frame (60 fps); speaking above 0.12; onset on rising past 0.35 after dipping below 0.2.
- Config `[ui] overlay = true` by default.
- Commits: conventional, English. Branch `feat/44-recording-overlay`. PR body `Closes #44`.

## Review Focus

1. A new dictation starting during an exit animation must show immediately, never be hidden by the old fade (Task 2 test `new_phase_cancels_exit`).
2. Earlier jobs finishing while a new recording runs must not play an exit over it (Task 5 test `recording_wins_over_outcome`).
3. Turning the overlay off in settings and reloading must hide it and stop its thread; turning it on again must work without restarting the daemon (Task 5 `configure`, checked manually in Task 6).
4. Paste still lands in the original window while the overlay appears and animates, including in push-to-talk, where the overlay shows while keys are held (Task 6 manual check).
5. Mixed-DPI / moved taskbar: the overlay sits inside the work area of the monitor the user types on (Task 4 manual check on 100 % and 150 %).

---

### Task 1: Live level from capture

**Files:**
- Modify: `crates/hece-audio/src/capture.rs` (`CaptureConfig`, `Shared`, `Shared::push`, `Capture::start`, tests)
- Modify: `crates/hece-audio/src/lib.rs` (re-export `Level`)

**Interfaces:**
- Produces: `pub struct Level` (`#[derive(Debug, Clone, Default)]`, wraps `Arc<AtomicU32>` holding `f32` bits) with `pub fn get(&self) -> f32` (RMS of the latest chunk, linear 0..1) and `pub(crate) fn set(&self, rms: f32)`. `CaptureConfig` gains `pub level: Level` (default: a fresh `Level`). `hece_audio::Level` re-exported.

- [ ] **Step 1: Write the failing test** in `capture.rs` tests: `push_publishes_rms_level`: build `Shared` (extend the `shared()` helper with a `Level`), push `[0.5, -0.5, 0.5, -0.5]` mono f32, assert `(level.get() - 0.5).abs() < 1e-6`; push `[0.0; 4]`, assert `level.get() == 0.0`.
- [ ] **Step 2: Run** `cargo test -p hece-audio push_publishes_rms_level`. Expected: compile error (no `Level`).
- [ ] **Step 3: Implement** `Level`; store a clone in `Shared` (`level: Level`); in `push`, after `downmix`, compute RMS of `mono` and `set` it (Relaxed). In `Capture::start`, reset the level to 0 before opening. Pass `cfg.level.clone()` into `Shared`.
- [ ] **Step 4: Run** `cargo test -p hece-audio`. Expected: all pass.
- [ ] **Step 5: Commit** `feat(audio): publish live input level`.

### Task 2: Overlay phase machine and level meter (pure)

**Files:**
- Modify: `Cargo.toml` (workspace dep `tiny-skia = { version = "0.12", default-features = false, features = ["std", "simd"] }`)
- Modify: `crates/hece-platform/Cargo.toml` (`tiny-skia.workspace = true` in the platform-independent `[dependencies]`)
- Create: `crates/hece-platform/src/overlay/mod.rs` (types, re-exports)
- Create: `crates/hece-platform/src/overlay/meter.rs`
- Create: `crates/hece-platform/src/overlay/scene.rs` (state only in this task; drawing in Task 3)
- Modify: `crates/hece-platform/src/lib.rs` (`pub mod overlay;`)

**Interfaces:**
- Produces (in `hece_platform::overlay`):
  - `#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum Outcome { Inserted, NothingHeard, NotPasted, Failed }`
  - `#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum Phase { Idle, Opening, Listening, Transcribing, Finished(Outcome) }`
  - `Phase::to_raw(self) -> usize` / `Phase::from_raw(usize) -> Phase` (unknown → `Idle`), for posting as `wParam`.
  - `pub struct Voice { pub level: f32, pub speaking: bool, pub onset: bool }`
  - `#[derive(Debug, Default)] pub struct Meter` with `pub fn update(&mut self, rms: f32) -> Voice` (one call per frame).
  - `#[derive(Debug)] pub struct Scene` with `pub fn new(reduced_motion: bool) -> Self`, `pub fn set_phase(&mut self, phase: Phase, now: Duration)`, `pub fn tick(&mut self, now: Duration, rms: f32) -> bool` (advances easing, waves, meter; returns whether the overlay should be visible), `pub fn visible(&self) -> bool`. `now` is time since the overlay started.
  - `pub const EXIT: Duration = Duration::from_millis(300);`

Phase rules (from the spec): `Opening`/`Listening`/`Transcribing` while hidden or exiting → visible, exit cancelled. `Finished(o)` → exit with that outcome, hidden `EXIT` later. `Idle` while visible and not exiting → exit as `NothingHeard` (silver fade). `Idle` or the same `Finished` while already exiting → no change. Waves are spawned only on `Voice::onset` in `Listening`; colour weights ease toward the phase target at 0.07 per frame (prototype `stepW`).

- [ ] **Step 1: Write failing tests** in `meter.rs`:
  - `silence_is_not_speaking`: 30 updates with `0.0` → `level < 0.01`, `!speaking`, no onset.
  - `speech_maps_db_to_level`: 60 updates with `0.1` (−20 dBFS) → `(level - 30.0/35.0).abs() < 0.02`, `speaking`.
  - `onset_fires_once_per_syllable`: rising `0.1` frames give exactly one `onset`; after 60 frames of `0.0` and rising again, a second one.
- [ ] **Step 2: Write failing tests** in `scene.rs`:
  - `starts_hidden`: `Scene::new(false).tick(0ms, 0.0) == false`.
  - `finished_hides_after_exit`: Listening at 0 ms, `Finished(Inserted)` at 1000 ms; `tick(1000 + 299 ms)` true, `tick(1000 + 301 ms)` false.
  - `new_phase_cancels_exit`: `Finished(Failed)` at 1000 ms, `Opening` at 1100 ms; `tick(1500 ms)` true.
  - `idle_while_visible_fades_out`: Listening, then `Idle` at 500 ms; `tick(801 ms)` false.
  - `repeated_finished_does_not_restart_exit`: `Finished(Inserted)` at 1000 ms and again at 1200 ms; `tick(1301 ms)` false.
- [ ] **Step 3: Run** `cargo test -p hece-platform overlay`. Expected: compile errors.
- [ ] **Step 4: Implement** `Meter` (dB mapping and smoothing from Global Constraints; onset arm/disarm) and the `Scene` state (phase, exit deadline, colour weights, waves list with start times, rotation angle; when `reduced_motion` the angle stays fixed and the deformation amplitude is 0).
- [ ] **Step 5: Run** `cargo test -p hece-platform`. Expected: all pass.
- [ ] **Step 6: Commit** `feat(platform): overlay phase machine and level meter`.

### Task 3: Draw the A3 sphere

**Files:**
- Create: `crates/hece-platform/src/overlay/sphere.rs` (icosphere geometry: subdivision 2 → 162 vertices, 480 edges, built once)
- Modify: `crates/hece-platform/src/overlay/scene.rs` (`draw`)

**Interfaces:**
- Consumes: `Scene` from Task 2.
- Produces: `Scene::draw(&self, pixmap: &mut tiny_skia::Pixmap)`. The pixmap is square; `scale = pixmap.width() as f32 / 140.0` maps the prototype's 140-px coordinates. Draws nothing when not visible (pixmap cleared to transparent).

Port from the prototype (A3): `project` (base radius 18 → 30 as the ready weight drops, lobes `sin(x*2.2+t*1.6)*cos(y*2.4-t*1.2) + 0.6*sin(z*3.1+t*2.3)` times amplitude `ready*1.2 + silent*2 + speak*(4+14*level)` times 0.6, waves `exp(-((d-age*1.6)^2)*10)*(1-age/1.4)` along `d = acos(z)`, +7 px per wave, breathing `ready*sin(2t)*1.3`, rotation `ry = 0.45t`, `rx = 0.4`), edge colour = mix(base, rainbow hue `atan2(ny,nx)*57.3 + 60t` at s 0.95 l 0.62, `min(1, wave*1.4 + speak*level*0.25)`), alpha `0.18 + 0.5*depth + 0.35*wave` capped 0.95, line width 0.75, vertex dots where `wave > 0.15`, core radius `3.5 + speak*level*5 + ready*sin(2t)*0.6` with a radial glow 4× its radius. Palette: violet `#9682ff`, light blue `#5ab4e6`, silver `#a8acb6`, danger `#b3261e`, amber `#f5a524`.
New looks the prototype lacks (spec "States"): **Transcribing**: no waves, all edges carry the rainbow at weight 0.5 with hue speed 30 °/s, radius 0.85×, core pulsing `sin(3t)`. **Exit**: radius and alpha scale by `1 - k` over `EXIT` (`k` 0→1); `Inserted` also pulls vertices toward the core (`r *= 1 - k`); `Failed` tints the core `danger`, `NotPasted` amber, `NothingHeard` silver. **Opening**: scale from 0.6 to 1 over 150 ms.

- [ ] **Step 1: Write failing tests:** in `sphere.rs` `icosphere_has_expected_size` (162 vertices, 480 edges, all vertices unit length within 1e-5). In `scene.rs` `listening_draws_something`: Listening, tick a few frames with `rms = 0.1`, draw into a 140×140 pixmap → some pixel alpha > 0; `hidden_draws_nothing`: fresh scene → all alpha 0.
- [ ] **Step 2: Run** `cargo test -p hece-platform overlay`. Expected: fail.
- [ ] **Step 3: Implement** geometry and `draw` with tiny-skia (`Paint` anti-aliased, `Stroke` per edge path, `RadialGradient` for the glow).
- [ ] **Step 4: Run** `cargo test -p hece-platform`. Expected: pass.
- [ ] **Step 5: Commit** `feat(platform): draw the overlay sphere`.

### Task 4: The Windows overlay window

**Files:**
- Create: `crates/hece-platform/src/windows/overlay.rs` (FFI module, `#![allow(unsafe_code, reason = "Win32 layered window FFI wrapper")]`)
- Modify: `crates/hece-platform/src/windows/mod.rs`, `crates/hece-platform/src/lib.rs` (export `Overlay`; `PlatformError::Overlay` with code `"overlay_failed"`)
- Modify: `crates/hece-platform/Cargo.toml` (windows-sys features `Win32_Graphics_Gdi`, `Win32_UI_HiDpi`)
- Create: `crates/hece-platform/examples/overlay.rs` (manual check: cycles Opening → Listening with a synthetic level → Transcribing → `Finished(Inserted)` → … every few seconds)

**Interfaces:**
- Consumes: `Scene`, `Phase`, `EXIT` from Tasks 2–3.
- Produces: `#[derive(Debug)] pub struct Overlay` with `pub fn start(level: impl Fn() -> f32 + Send + 'static) -> Result<Self, PlatformError>` (spawns thread `hece-overlay`, creates the hidden window, returns once ready, like `Desktop::start`) and `pub fn set_phase(&self, phase: Phase)` (posts `WM_APP + 1` with `phase.to_raw()`; duplicates are fine). `Drop` posts `WM_QUIT` and joins.

Thread behaviour: `SetThreadDpiAwarenessContext(PER_MONITOR_AWARE_V2)` first. Window class with `WM_NCHITTEST → HTTRANSPARENT`, `WM_MOUSEACTIVATE → MA_NOACTIVATE`, `WM_TIMER` → frame. On a phase message while hidden: read `SPI_GETCLIENTAREAANIMATION` (reduced motion), place on the monitor of `GetForegroundWindow()` (`MonitorFromWindow` + `GetMonitorInfoW` work area + `GetDpiForMonitor`), `ShowWindow(SW_SHOWNOACTIVATE)`, `SetTimer(16 ms)`. Each frame: `scene.tick(now, level())`; if it returns false → `KillTimer` + `SW_HIDE`; else `scene.draw` into a pixmap sized `140 * dpi / 96`, copy RGBA→BGRA (both premultiplied) into a top-down 32-bit DIB section, `UpdateLayeredWindow` with `AC_SRC_ALPHA`. Recreate the DIB only when the size changes.

- [ ] **Step 1: Implement** the module and the example (no unit tests: this is Win32 glue; the logic it drives is tested in Tasks 2–3).
- [ ] **Step 2: Run** `cargo clippy -p hece-platform --all-targets -- -D warnings`. Expected: clean.
- [ ] **Step 3: Manual check** `cargo run -p hece-platform --example overlay` with Notepad focused and typing: the caret never leaves Notepad, clicks on the sphere reach Notepad, no taskbar button, not in Alt+Tab, sphere top-right of the Notepad monitor; repeat at 150 % scale. Measure CPU while visible and while hidden (Task Manager, the example's process). Write the numbers down for the PR.
- [ ] **Step 4: Commit** `feat(platform): layered click-through overlay window on Windows`.

### Task 5: Wire the daemon and the config

**Files:**
- Modify: `crates/hece/src/config.rs` (`UiConfig { overlay: bool }`, default `true`, `ui` field on `Config`, `[ui]` block in `TEMPLATE`, tests)
- Modify: `crates/hece/src/run.rs`

**Interfaces:**
- Consumes: `hece_audio::Level` (Task 1), `hece_platform::{Overlay, overlay::{Phase, Outcome}}` (Tasks 2, 4).
- Produces: `fn overlay_phase(recording: bool, mic_live: bool, pending: usize, outcome: &mut Option<Outcome>) -> Phase` in `run.rs`: recording && !mic_live → `Opening`; recording → `Listening`; pending > 0 → `Transcribing`; else `outcome.take()` → `Finished(o)`, else `Idle`.

Daemon changes: one `Level` created in `execute`, put into `capture_config` (keep it across `configure`). `Daemon` gains `overlay: Option<Overlay>` (started when `cfg.ui.overlay`; on `configure`, start or drop to match; a start failure is logged with its code and dictation continues) and `outcome: Option<Outcome>`. Set `outcome` where jobs end: `insert` (`Pasted` → `Inserted`, `FocusChanged` → `NotPasted`, errors → `Failed`), `Action::NothingHeard` → `NothingHeard`, `Action::Failed` → the typed kind stored with the failure (change `failures` to `HashMap<RecordingId, (Outcome, String)>`; empty transcript → `NothingHeard`, provider error → `Failed`), `Action::Abort` and `Action::DiscardCapture` → `Failed`. After `refresh_tray()` in the loop, `set_phase(overlay_phase(...))` when the overlay exists.

- [ ] **Step 1: Write failing tests:** config `ui_overlay_defaults_on` (config without `[ui]` → `true`) and `ui_overlay_can_be_off` (`[ui]\noverlay = false` → `false`), alongside existing config tests. In `run.rs` tests: `recording_wins_over_outcome` (`recording=true, mic_live=true, outcome=Some(Inserted)` → `Listening`, outcome still `Some`); `outcome_plays_once_when_all_done` (`false, false, 0, Some(Failed)` → `Finished(Failed)`, then `Idle`); `pending_shows_transcribing` (`false, false, 1, Some(Inserted)` → `Transcribing`, outcome kept).
- [ ] **Step 2: Run** `cargo test -p hece`. Expected: fail.
- [ ] **Step 3: Implement** as described.
- [ ] **Step 4: Run** the quality gate: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`. Expected: all green.
- [ ] **Step 5: Commit** `feat: show the recording overlay from the daemon`.

### Task 6: Settings toggle, design note, verification, PR

**Files:**
- Modify: `apps/settings/src-tauri/src/main.rs` (`KEYS` += `"ui.overlay"`)
- Modify: `apps/settings/src/lib/Shortcuts.svelte` (a toggle row after "Mikrofon": label "Kayıt göstergesi", help "Kayıt sırasında ekranın sağ üstünde sese tepki veren küre", key `ui.overlay`, default checked when unset)
- Modify: `DESIGN.md` (new section "Recording overlay": the accent exception and the palette from Task 3)

- [ ] **Step 1: Implement** the row with the existing toggle style (square checkbox, DESIGN.md) and the `change(row, {"ui.overlay": checked})` pattern.
- [ ] **Step 2: Run** in `apps/settings`: `bun run check`. Expected: 0 errors.
- [ ] **Step 3: Manual check, real app** (`cargo run -p hece`): dictate into Notepad and a browser text field (toggle and hold modes); paste lands every time; overlay shows Opening → Listening (rainbow waves on speech, silver on pauses) → Transcribing → collapse; cancel shows the danger fade; a second dictation during transcription shows the live recording; switch the toggle off in settings → overlay gone without restart, on again → back. Screenshots on a light and a dark desktop.
- [ ] **Step 4: Run** the full quality gate again. Expected: green.
- [ ] **Step 5: Commit** `feat(settings): toggle for the recording overlay`, push, open the PR to `main` with `Closes #44`, CPU numbers and screenshots.
