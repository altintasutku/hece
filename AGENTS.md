# Hece

Hece is an open-source desktop voice-dictation app. A global hotkey starts recording, speech is
transcribed (local Whisper via CTranslate2, or a cloud provider), passed through quality layers
(vocabulary, hallucination filter, confidence signal, optional guarded LLM cleanup), and inserted
into the focused window. Windows first; Linux and macOS later.

## Layout

| Path | Role |
| --- | --- |
| `crates/hece-core` | Pure, I/O-free logic: vocabulary, hallucination filter, confidence, negation guard, recording state machine, shared types |
| `crates/hece-engine` | Local Whisper (CTranslate2); lib + bin; feature `cuda` (off by default) |
| `crates/hece-providers` | `Provider` trait, STT adapters, API key handling |
| `crates/hece-audio` | Microphone capture (cpal) and resampling to 16 kHz mono; needs `libasound2-dev` on Linux |
| `crates/hece-platform` | Windows: global hotkey, clipboard + paste, focused window |
| `crates/hece` | Daemon + CLI binary |
| `apps/settings` | Settings window: Tauri 2 (`src-tauri`, crate `hece-settings`, Windows-only) + Svelte 5/Vite; edits `config.toml` via `hece::settings`, then asks the daemon to `reload`. Visual rules in `DESIGN.md` |

## Commands

Quality gate (must be green before any PR):

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

- Run the daemon/CLI: `cargo run -p hece`
- Settings app (in `apps/settings`, bun from `mise.toml`): `bun install`, `bun run check` (types + Svelte),
  `bunx tauri dev` (window against the Vite dev server), `bunx tauri build --no-bundle` (release exe);
  `bun run dev` alone renders in a browser with a mocked backend (`src/lib/devmock.ts`, for screenshots)
- Turkish model (CTranslate2, into `.cache/models`): `uv run scripts/prepare_model.py`
- Engine with CUDA on Windows: `powershell -File scripts/build-engine.ps1` (release `hece` + `hece-engine`; +crt-static, onnxruntime.dll loaded at run time)
- Engine with CUDA on Linux: `CUDA_ARCH_LIST="7.5;8.0;8.9;12.0" CMAKE_BUILD_PARALLEL_LEVEL=4 CARGO_BUILD_JOBS=4 cargo build -p hece-engine --features cuda-dynamic`
  (never two CUDA builds at once; details in `crates/hece-engine/README.md`)
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
- `unsafe` outside FFI wrapper modules (those need `#[allow(unsafe_code, reason = "...")]`).
- `unwrap()` / `expect()` outside tests.
- `#[allow(...)]` without `reason = "..."` (enforced by `clippy::allow_attributes_without_reason`); every `unsafe` block needs a `// SAFETY:` comment (`clippy::undocumented_unsafe_blocks`).
- Debug prints (`dbg!`, stray `println!`/`eprintln!`). User-facing CLI output goes through one output module that allows `clippy::print_stdout` with a `reason`.
- Low-level input hooks and injection beyond one paste chord: `WH_KEYBOARD_LL`, evdev, uinput, ydotool. Global shortcuts use `RegisterHotKey` (Windows) or the desktop portal (Linux).
- Key-by-key typing. Insertion is clipboard + one paste chord.

### Disk hygiene
A debug build of this workspace is several GB and every worktree builds its own `target`; leftovers
once filled ~135 GB in two days (#99).
- `scripts/dev-cleanup.ps1` runs at session start (Claude Code: `.claude/settings.json`; Codex:
  `.codex/hooks.json`, needs a one-time trust in `/hooks`). It removes worktrees and branches whose
  PR is merged or closed, detached worktrees already on `origin/main` and idle for a day, stale git
  temp objects, and an oversized `target/debug/incremental`.
  Run it by hand after merging; `-DryRun` shows what it would do.
- A finished task ends with a PR. When the session start lists open PRs with green checks, tell the
  user they are waiting for a merge; merge only when the user says so.
- After a PR is merged, remove its worktree (`git worktree remove`) instead of leaving it.
- Do not change `[profile.dev]` debug settings back to full debug info, and do not copy `target`
  between worktrees.
- CUDA engine builds (`scripts/build-engine.ps1`) go to `target/release`; cleanup never touches it.

### Conventions
- Every crate has `[lints] workspace = true`.
- Branches: `feat/<issue>-<slug>` (or `fix/...`). Conventional commits in English.
- PR body includes `Closes #<n>`.
