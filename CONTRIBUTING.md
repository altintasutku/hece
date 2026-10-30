# Contributing to Hece

## Prerequisites

- Rust stable, 1.94 or newer
- CUDA toolkit, only if you build `hece-engine` with `--features cuda`

## Quality gate

Run this before opening a PR; CI runs the same commands:

```sh
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
```

Do not use `--all-features`; the `cuda` feature requires CUDA.

## Commits and branches

- Conventional commits (`feat:`, `fix:`, `docs:`, `ci:`, `chore:` ...), written in English.
- Branch names: `feat/<issue>-<slug>` or `fix/<issue>-<slug>`.

## Privacy

Never commit real recordings, transcripts, personal vocabulary, or API keys. Test fixtures must be
synthetic. Benchmark data is passed as a path outside the repository.

## Pull requests

- Link the issue with `Closes #<n>` in the description.
- Keep the change focused and the quality gate green.
- Add tests for new behavior; see `CLAUDE.md` for the project rules (error handling, forbidden patterns).
