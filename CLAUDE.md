# Hece

Hece is an open-source desktop voice-dictation app. A global hotkey starts recording, speech is
transcribed (local Whisper via CTranslate2, or a cloud provider), passed through quality layers
(vocabulary, hallucination filter, confidence signal, optional guarded LLM cleanup), and inserted
into the focused window. Linux (KDE Plasma Wayland) first; macOS and Windows later.

## Layout

| Path | Role |
| --- | --- |
| `crates/hece-core` | Pure, I/O-free logic: vocabulary, hallucination filter, confidence, negation guard, shared types |
| `crates/hece-engine` | Local Whisper (CTranslate2); lib + bin; feature `cuda` (off by default) |
| `crates/hece-providers` | `Provider` trait, STT adapters, API key handling |
| `crates/hece-platform` | `Platform` trait + Linux impl: portal shortcuts, paste, clipboard, window class |
| `crates/hece` | Daemon + CLI binary |
| `apps/settings` | Tauri 2 settings UI (not created yet) |

## Commands

Quality gate (must be green before any PR):

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

- Run the daemon/CLI: `cargo run -p hece`
- Engine with CUDA: `CUDA_ARCH_LIST="8.9;12.0" cargo build -p hece-engine --features cuda`
- Never use `--all-features`; `cuda` needs a CUDA toolchain and CI does not have one.

## Rules

### Privacy
- No personal data, recordings, transcripts, or personal vocabulary in the repo, issues, or commits.
- Test fixtures are synthetic. Benchmark data is passed as a path outside the repo.

### Errors and logging
- `thiserror` error enums per crate; `anyhow` only in a bin's `main`.
- The user sees a short message plus a stable error code. Details go to structured logs.
- Audio, transcripts, and API keys are never logged.

### Forbidden
- `unsafe` outside FFI wrapper modules (those need `#[allow(unsafe_code)]` with a justifying comment).
- `unwrap()` / `expect()` outside tests.
- `#[allow(...)]` without an explanatory comment.
- Debug prints (`println!`, `eprintln!`, `dbg!`) in non-test code.
- Kernel input access: evdev, uinput, ydotool. Global shortcuts go through the desktop portal.
- Key-by-key typing. Insertion is clipboard + one paste chord.

### Conventions
- Every crate has `[lints] workspace = true`.
- Branches: `feat/<issue>-<slug>` (or `fix/...`). Conventional commits in English.
- PR body includes `Closes #<n>`.
