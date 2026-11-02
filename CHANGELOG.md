# Changelog

All notable changes to Hece are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

The first public release (v0.1.0) is being prepared. Windows only.

### Added

- Background voice dictation: hold **Ctrl+Shift+Space** and speak, release to insert the text into
  the focused window; a toggle mode starts and stops on a press. **Ctrl+Shift+Backspace** cancels.
- Insertion through the clipboard and a single paste; your previous clipboard is restored, and
  transcripts stay out of Win+V history and the cloud clipboard.
- Local transcription with Whisper on CTranslate2 (NVIDIA GPU), no Python needed. The model runs
  in a separate worker that loads it in the background, so the first dictation does not wait, and
  releases it after a period of inactivity.
- Multilingual Whisper large-v3 turbo as the default model, with language auto-detection; a
  fine-tuned Turkish model is also available.
- One-click model download from the settings window or with `hece model download`: pinned
  revisions, sha256-checked, resumable.
- Cloud providers: any OpenAI-compatible speech-to-text endpoint (OpenAI, Groq, self-hosted
  servers) and Deepgram, with fallback between providers. API keys are kept in the Windows
  Credential Manager.
- Quality layers: personal vocabulary, a hallucination filter, a low-confidence signal, and an
  optional LLM cleanup (OpenAI-compatible or Anthropic) guarded against changed meaning such as
  dropped negations or added lines.
- Long recordings are transcribed in chunks while you are still speaking, so the text is ready
  soon after you stop.
- Tray icon that shows the state (idle, recording, transcribing) and has the menu.
- Animated recording overlay in the corner of the screen that follows your voice level; it can be
  turned off in settings.
- Settings window (**Hece Settings**) for the provider, model, shortcuts and options. It edits
  `config.toml` without losing your comments and applies changes without a restart. It opens by
  itself on first run.
- English interface (settings window, tray menu, notifications) with a Turkish translation that is
  used when the Windows display language is Turkish.
- The settings window shares the website's dark look (Gece).
- Local OpenAI-compatible transcription API on `127.0.0.1`, off by default and protected by a
  token, so other apps can share the loaded model.
- Command line: `hece run`, `status`, `toggle`, `history`, `repaste`, `key`, `autostart`,
  `config`, `devices`, `model`, `dictate` and `bench` (accuracy and latency benchmarks).
- Dictation history and notifications for errors, each with a short message and a stable error
  code.
- Per-user Windows install and uninstall scripts (no administrator rights), Start menu entries and
  optional autostart. The release zip can include the CUDA runtime.

### Security

- The daemon control pipe and the engine worker pipe only accept the current Windows user.
- The local API binds to loopback only, checks the `Host` header and requires a token.
- Audio, transcripts and API keys are never written to logs.
- Network requests to providers have connect, header and upload timeouts.

[Unreleased]: https://github.com/altintasutku/hece/commits/main
