# Recording overlay: design

Date: 2026-11-02. Status: draft, waiting for approval.

## Intent

While Hece records, a small animated sphere floats in the top-right corner of the screen, above every window. It
tells the user "I am listening, and I hear you" without looking at the tray icon. It reacts to the voice: when the
user speaks it comes alive with colour, when they pause it calms down, when they stop it shows that the text is
being produced, then it disappears.

The reference is the voice mode of ChatGPT and similar dictation apps (a wireframe sphere that turns red and
deforms with speech, grey when silent). We do not copy it one to one: the sphere carries Hece's own mark, the
syllable dot (*hece* = syllable), and it moves with the rhythm of speech instead of a constant jitter.

The look was chosen with Utku on 2026-11-02 from live mockups: direction **A3, "hece + rainbow"**. The working
prototype is `2026-11-02-recording-overlay-prototype.html` next to this file (open it in a browser; the A3 card and
`drawA3`/`project` are the reference implementation of the look).

## The look (A3), decided

- A slowly rotating wireframe icosphere (subdivision 2: 162 vertices, 480 edges), drawn with thin anti-aliased
  lines. Edges facing the viewer are brighter than the ones behind (depth-based alpha).
- A solid **syllable dot** in the centre of the sphere, with a soft glow.
- **Speaking:** the mesh deforms gently with the level. Every syllable onset (level rises past a threshold after
  having dipped) sends a **wave** across the sphere surface. Where a wave passes, edges and vertex dots are painted
  in rainbow colours (hue from the edge's angle around the sphere plus time), then fade back to violet. The core
  grows with the level and cycles through rainbow hues.
- **Recording but silent:** colours fade to a silver-violet, the mesh calms down, no waves.
- **Speaking again:** waves and rainbow come back.
- Colour changes are smooth (weights per state eased each frame, as in the prototype), never a hard cut.

## States and behaviour

The overlay is **hidden whenever Hece is idle.** It only exists on screen during one dictation.

| Daemon situation | Overlay |
| --- | --- |
| Hotkey pressed, mic opening (not yet live) | Appears with a short entry: small calm sphere (blue-violet), scales up. *Builder's/our call: this replaces the reference's "idle blue ball", since Hece never listens while idle.* |
| Mic live, user speaking | Speaking look (above) |
| Mic live, user silent | Silent look (above) |
| Recording stopped, transcribing | *Our call:* no waves; a slow rainbow flow over the whole mesh, a slightly smaller sphere, the core pulsing at a calm rate. Shows "working". |
| Text inserted | *Our call:* the sphere collapses into the core and fades out (~300 ms), then hides. |
| Nothing heard | *Our call:* fades to silver and shrinks out (~300 ms). The existing toast still explains it. |
| Failed / aborted / cancelled | *Our call:* a short `--danger` tint on the core, then the same fade-out. The existing toast carries the code. |

| Text not pasted, focus changed (`Inserted::FocusChanged`, text left on the clipboard) | *Our call:* a short amber tint on the core, then the silver fade-out. It must not look like the success collapse. Low-confidence and cleanup-skipped warnings still inserted text, so they get the success collapse. |

The daemon already knows all of these (see "Where it hooks in"). A new dictation that starts during a fade-out
cancels the fade and goes straight to the entry state (a generation counter or equivalent so a late hide never
closes a newer session).

**Overlapping dictations** (*our call*). The recorder lets a new recording start while the previous one is still
transcribing, and results are inserted in recording order. The overlay is driven by the daemon's aggregate state,
the same inputs `refresh_tray()` uses (`is_recording`, `mic_live`, `pending()`), plus the outcome of the last
finished job:

- A live recording always wins: while one is in progress, the overlay shows it (entry / speaking / silent), and
  outcomes of earlier jobs do not animate on top of it. Their result is still reported by the toast and the tray.
- With no recording and `pending() > 0`, it shows the transcribing look.
- The end animation (collapse, silver fade, amber or danger tint) plays only when there is no recording and
  `pending() == 0`, using the outcome of the job that finished last.
- Cancel with jobs still pending: if anything is left, stay in the transcribing look; if nothing is left, play the
  danger fade.

## Hard requirements

1. **Never steals focus.** Hece pastes into the focused window; if the overlay ever activates, the text goes to the
   wrong place. The window is never activated, not on create, not on show, not on click.
2. **Click-through.** Mouse input passes to whatever is behind it.
3. **Not in the taskbar, not in Alt+Tab.**
4. **Fast:** visible within ~one frame of the daemon entering the capture state (target < 50 ms after the hotkey).
5. **Zero idle cost:** when hidden, no timer, no rendering, no wake-ups.
6. **Cheap while visible:** target well under 1 % of one core at 60 fps on a modern CPU (measure it).
7. **DPI correct:** crisp and the right size on any monitor scale, including mixed-DPI setups.
8. **Privacy:** only a scalar level crosses from capture to the overlay. No audio buffer is shared with it, and
   levels are never logged.

## How it is built, decided

Research (2026-11-02; summary in the GitHub issue) compared a Tauri/WebView2 overlay with a native window.
Decision: **a native Win32 layered window inside the daemon process**, not Tauri.

Why: Tauri's overlay path has a failure mode for each requirement above. `show()` uses `SW_SHOW` after the first
show (focus), `skip_taskbar` leaks back after an Explorer restart, click-through on transparent always-on-top windows
has an open composition race (tauri#15947), transparent windows flash white on show, and a WebView2 process tree
would stay resident in the background (Handy, the closest prior art, has open issues for each of these, including
CPU burn in Modern Standby). It would also need either Tauri inside the daemon or a third process plus a streaming
IPC. The native window plugs straight into the daemon's existing Win32 message loop and reads the level from an
atomic.

Shape of the solution:

- **Window:** `WS_POPUP` with `WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_NOACTIVATE |
  WS_EX_TOOLWINDOW`; `WM_NCHITTEST` returns `HTTRANSPARENT`; `WM_MOUSEACTIVATE` returns `MA_NOACTIVATE`. Shown with
  `ShowWindow(SW_SHOWNOACTIVATE)` (or `SetWindowPos(... SWP_NOACTIVATE | SWP_SHOWWINDOW)`), hidden with `SW_HIDE`.
  Created once at daemon start, hidden, and reused.
- **Drawing:** each frame is rendered on the CPU with **tiny-skia** into a premultiplied BGRA pixmap, copied into a
  DIB section and pushed with `UpdateLayeredWindow` (per-pixel alpha). At 140×140 logical px this is tiny.
  *Pending Utku's approval: tiny-skia is a new dependency.*
- **Thread:** the overlay runs on its own thread with its own message loop (keeps the 60 fps timer off the hotkey
  and tray thread). The daemon talks to it the same way it talks to the desktop thread: posting `WM_APP + n`
  messages (state changes). The thread drives frames with a timer only while visible.
- **Level:** `hece-audio` computes the RMS of each chunk in `Shared::push` (right after downmix) and stores it as
  `f32` bits in an `AtomicU32`. The overlay reads it each frame. Mapping and smoothing as in the prototype: dBFS
  mapped from about −50..−15 dB to 0..1, fast attack (~0.35 per frame), slow release (~0.08), "speaking" when the
  smoothed level is above ~0.12, onset when it crosses ~0.35 upward after having dropped below ~0.2. Tune these on
  real voices; they are starting points, not decisions.
- **Placement:** top-right of the work area of the monitor that holds the foreground window at the moment
  recording starts, with a margin of 16 logical px. Size 140×140 logical px. The overlay thread is per-monitor DPI
  aware (V2) via `SetThreadDpiAwarenessContext`, so the daemon's own DPI mode does not matter. The size and position
  are recomputed on every show.
- **Reduced motion:** if Windows "Animation effects" is off (`SPI_GETCLIENTAREAANIMATION`), the sphere does not
  rotate or deform; colour still follows the state.
- **Code layout:** the FFI window code lives in `hece-platform/src/windows/` as its own module with
  `#![allow(unsafe_code, reason = "...")]`, like `desktop.rs`. The scene (sphere geometry, waves, colour weights,
  level mapping, state transitions) is pure Rust with no Win32 types, so it can be unit-tested and reused on Linux
  and macOS later. Exactly which crate holds the scene is the builder's call; `hece-core` must stay free of the
  rendering dependency.

## Where it hooks in (current code)

- Daemon event loop: `crates/hece/src/run.rs`, `execute` and `Daemon::act`. `refresh_tray()` already derives
  Recording / Processing / warning from the daemon state after every message; the overlay state is derived at the
  same point. The last job's outcome is recorded where it is decided: `insert()` (pasted, `FocusChanged`, error)
  and `fail()`. Nothing-heard vs failed is only told apart by the `failures` map today; the outcome passed to the
  overlay should carry that distinction as a type, not text.
- Audio: `crates/hece-audio/src/capture.rs`, `Shared::push`.
- Desktop thread pattern (posting `WM_APP + n`): `crates/hece-platform/src/windows/desktop.rs`.

## Setting

- New config key `[ui] overlay = true` (default on), with the same serde pattern as the other sections, the
  commented block in the config template, applied at start-up and on `reload`.
- The settings app gets one toggle row for it (allowlist `ui.overlay` in `apps/settings/src-tauri/src/main.rs`,
  Turkish strings, `DESIGN.md` rules). Place it in an existing section unless it reads wrong there; builder's call.
- Note: configs are parsed with `deny_unknown_fields`, so an older binary rejects a config that has `[ui]`. This is
  the existing behaviour for every new section; accept it.

## Design system

`DESIGN.md` says the accent is used sparingly and never for decoration. The overlay is a deliberate exception: it
lives outside the settings window, on the user's desktop, and its job is to be noticed. Record this in `DESIGN.md`
with the overlay's palette (blue-violet entry, violet base, silver silent, rainbow on waves, `--danger` for failure).

## Goals

- The user always knows, at a glance, that Hece is recording, hears them, and is working on the text.
- It feels like Hece's own, not a copy.

## Non-goals (for this change)

- Linux and macOS overlays (keep the scene portable; the window comes later).
- Dragging, choosing the corner, size settings, themes.
- Buttons on the overlay (cancel etc.); it is click-through by design.
- Showing live transcript text.

## Anti-goals (would make it a failure even if it "works")

- Any case where the paste lands in the wrong window because of the overlay.
- The overlay in the taskbar, in Alt+Tab, or catching clicks.
- Noticeable CPU or battery use while idle, or a resident browser process.
- A visible delay between the hotkey and the sphere, or a white/black flash on show.
- Audio or levels written to logs.

## Testing and verification

- Unit tests for the pure parts only where they can catch real bugs: the state transitions (including a new
  dictation during fade-out) and the level mapping/onset detection. No tests for drawing.
- Manual, on the real app, recorded in the PR: dictate into Notepad and a browser text field with the overlay on;
  paste lands correctly every time; the overlay never appears in the taskbar or Alt+Tab; clicks pass through; the
  look on a light and a dark desktop, on 100 % and 150 % scale; CPU while visible and while idle (Task Manager or
  `typeperf`), with numbers in the PR.
- Quality gate as usual: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace`, and `bun run check` for the settings change.

## Left to the builder

The exact crate for the scene, module names, how frames are timed (`SetTimer` vs a waitable timer), how the level
atomic is shared (handle passed at start-up vs per capture), and the exact easing constants, as long as the
requirements above hold.
