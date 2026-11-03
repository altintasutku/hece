# Contributing to Hece

Thanks for helping. Hece is a background voice-dictation app: a global hotkey records, speech is
transcribed (local Whisper on CTranslate2, or a cloud provider), passed through quality layers, and
pasted into the focused window. Windows is the only supported platform today; Linux and macOS ports
are welcome (see the issues labeled `help wanted`).

Questions and ideas go to [Discussions](https://github.com/altintasutku/hece/discussions); bugs and
concrete feature requests to [issues](https://github.com/altintasutku/hece/issues). By taking part
you agree to the [Code of Conduct](CODE_OF_CONDUCT.md).

## Repository layout

| Path | Role |
| --- | --- |
| `crates/hece-core` | Pure, I/O-free logic: vocabulary, hallucination filter, confidence, negation guard, recording state machine, shared types |
| `crates/hece-engine` | Local Whisper on CTranslate2; library + worker binary; feature `cuda` (off by default) |
| `crates/hece-providers` | `Provider` trait, speech-to-text adapters, LLM cleanup, API key handling |
| `crates/hece-audio` | Microphone capture (cpal) and resampling to 16 kHz mono |
| `crates/hece-platform` | Desktop integration (Windows): global hotkey, clipboard + paste, tray, overlay, control pipe |
| `crates/hece` | Daemon + CLI binary (`hece`, and `hecew` without a console) |
| `apps/settings` | Settings window: Tauri 2 (`src-tauri`, crate `hece-settings`, Windows-only) + Svelte 5 / Vite. Visual rules in `DESIGN.md` |
| `scripts` | Packaging, installer, model preparation and publishing |

## Prerequisites

- **Windows 10 or 11.** The daemon and the settings app only run on Windows. The platform-independent
  crates also build and test on Linux (CI runs both; Linux needs `libasound2-dev`).
- **Rust 1.94** (pinned in `mise.toml`). Either install [mise](https://mise.jdx.dev) and run
  `mise install` in the repo, or install the same toolchain with
  [rustup](https://rustup.rs) (`rustup toolchain install 1.94`).
- **Visual Studio Build Tools** with the "Desktop development with C++" workload (MSVC linker).
- **bun** (version in `mise.toml`), only for the settings app. mise installs it too.
- **CUDA toolkit and cmake**, only if you work on the local engine with the `cuda` feature. Everything
  else, including CI, builds without CUDA.

## Build and run

```sh
cargo build --workspace
cargo run -p hece -- run        # the daemon in a console
cargo run -p hece -- --help     # all commands
```

Without a CUDA build of the engine, use a cloud provider in `config.toml` (`hece config` prints its
path).

Settings app (in `apps/settings`):

```sh
bun install
bun run check                 # types + Svelte
bunx tauri dev                # the window against the Vite dev server
bunx tauri build --no-bundle  # release exe
```

Engine with CUDA on Windows: `powershell -File scripts/build-engine.ps1` (release `hece` +
`hece-engine`). On Linux:
`CUDA_ARCH_LIST="7.5;8.0;8.9;12.0" CMAKE_BUILD_PARALLEL_LEVEL=4 CARGO_BUILD_JOBS=4 cargo build -p hece-engine --features cuda-dynamic`.
The CTranslate2 build is memory hungry; never run two CUDA builds at once. Details in
[`crates/hece-engine/README.md`](crates/hece-engine/README.md).

Release zip: `powershell -ExecutionPolicy Bypass -File scripts/package.ps1` (`-NoCuda` leaves the
CUDA runtime out).

## Quality gate

Every pull request must pass these; CI runs the same commands on Windows and Linux:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

If you changed the settings app, also run `bun run check` in `apps/settings`.

Never use `--all-features`: the `cuda` feature needs a CUDA toolchain, and CI does not have one.

## Code rules

### Privacy

- No personal data, recordings, transcripts or personal vocabulary in the repo, issues, commits or
  screenshots.
- Test fixtures are synthetic. Benchmark data is passed as a path outside the repo.

### Errors and logging

- One `thiserror` error enum per crate; `anyhow` only in a binary's `main`.
- Users see a short message plus a stable error code (each error enum has a `code()`). Details go
  to the structured log (`tracing`).
- Audio, transcripts and API keys are never logged.

### Forbidden

- `unsafe` outside FFI wrapper modules; those need `#[allow(unsafe_code, reason = "...")]`, and every
  `unsafe` block needs a `// SAFETY:` comment.
- `unwrap()` / `expect()` outside tests.
- `#[allow(...)]` without `reason = "..."`.
- Debug prints (`dbg!`, stray `println!` / `eprintln!`). User-facing CLI output goes through the one
  output module (`crates/hece/src/output.rs`).
- Low-level input hooks or injection beyond one paste chord: `WH_KEYBOARD_LL`, evdev, uinput,
  ydotool. Global shortcuts use `RegisterHotKey` on Windows and the desktop portal on Linux.
- Key-by-key typing. Insertion is clipboard + one paste chord.

Most of these are enforced by the workspace lints, so clippy will tell you.

### Conventions

- Every crate has `[lints] workspace = true`.
- Keep code and tests lean: add tests that would catch a real bug (new behavior, a fixed bug,
  error paths), not tests that restate the code.
- New dependencies: prefer well-maintained crates already in the workspace; say why in the PR.

## Branches, commits and pull requests

- Open or pick an issue first for anything larger than a small fix, so the approach can be agreed on.
- Branch names: `feat/<issue>-<slug>`, `fix/<issue>-<slug>` or `docs/<issue>-<slug>`.
- [Conventional commits](https://www.conventionalcommits.org) in English: `feat:`, `fix:`, `docs:`,
  `ci:`, `chore:`, `perf:`, `test:`, `refactor:`, optionally with a scope (`fix(platform): ...`).
- The PR description says what changed and why, and links the issue with `Closes #<n>`.
- Keep a PR focused on one thing, with the quality gate green.
- User-visible changes get a line under "Unreleased" in [`CHANGELOG.md`](CHANGELOG.md).

## Adding a model to the catalog

Downloadable models live in `CATALOG` in [`crates/hece/src/models.rs`](crates/hece/src/models.rs).
A model must be a Whisper model in CTranslate2 format, hosted on Hugging Face under a license that
allows redistribution.

1. Make sure the repo has the files the engine needs: `model.bin`, `config.json`,
   `tokenizer.json`, `vocabulary.json` and `preprocessor_config.json`. To convert and publish a
   fine-tuned model yourself, see `scripts/prepare_model.py` and `scripts/publish_model.py`; the
   latter prints the commit and each file's size and sha256.
2. Pin the exact commit: `revision` is the full 40-character commit hash, never a branch name.
3. For each file, record its size in bytes and its lowercase sha256 (for files stored with Git
   LFS, Hugging Face shows the sha256 on the file page). Verify by downloading and hashing the files
   yourself.
4. Add a `const` `Model` with a short `id` (it is the folder name and what users pick, so include
   the revision prefix, for example `large-v3-turbo-0a363e9`), a display `name`, the `repo`, and
   `languages` (empty for multilingual models). Add it to `CATALOG`; the first entry is the default.
5. `cargo test -p hece` checks that every entry is pinned and complete. Then run
   `cargo run -p hece -- model download <id>` and dictate with it to confirm it loads and transcribes.
6. In the PR, mention the model's license, size, and how it performed for you (`hece bench` on
   your own data, without sharing the data).

## Security issues

Do not open a public issue for vulnerabilities. Report them privately through the repository's
[Security tab](https://github.com/altintasutku/hece/security) → **Report a vulnerability**. See
[`SECURITY.md`](SECURITY.md) for what is in scope and what to include.

## License

Hece is MIT licensed. By contributing you agree that your contributions are licensed under the
same terms.
