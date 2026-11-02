# Hece

Background voice dictation for the desktop. Press a hotkey, speak, and the text lands in the focused window.

- Toggle and push-to-talk modes
- Switchable speech-to-text providers: local Whisper (CTranslate2, CUDA), OpenAI-compatible endpoints, Deepgram
- Quality layers: personal vocabulary, hallucination filter, low-confidence signal, optional guarded LLM cleanup
- Tiny when idle; the local model loads on demand and is released when idle
- Local OpenAI-compatible API so other apps can share one loaded model

Status: early development. Windows first; Linux and macOS later.

License: MIT

## Install (Windows)

1. Download `Hece_<version>_x64-setup.exe` from the
   [latest release](https://github.com/altintasutku/hece/releases/latest) and run it. It installs
   for the current user to `%LOCALAPPDATA%\Hece` (no administrator rights), adds **Hece** and
   **Hece Settings** to the Start menu, turns on autostart and opens the settings.
   The build is not code-signed yet, so Windows SmartScreen may warn: choose **More info → Run
   anyway**, or check `SHA256SUMS.txt` and the build attestation
   (`gh attestation verify <file> --repo altintasutku/hece`) first.
2. Local transcription needs an NVIDIA GPU and a Whisper model. Download one in **Hece Settings**
   or with `hece model download` (`hece model list` shows them): multilingual Whisper large-v3
   turbo by default, or a fine-tuned single-language model such as Turkish. Downloads are pinned
   and checksummed. Without an NVIDIA GPU, use a cloud provider instead.
3. In **Hece Settings**, pick the downloaded model, or a cloud provider and its key (stored in the
   Windows Credential Manager). It writes `config.toml` (`hece config` prints the path) and keeps
   your comments. If Hece starts before this is set, it opens the settings window itself.
   No NVIDIA GPU: [docs/cloud-providers.md](docs/cloud-providers.md) sets up Groq, OpenAI or Deepgram step by step.
4. Start Hece from the Start menu. Hold **Ctrl+Shift+Space** and speak; release to insert.
   **Ctrl+Shift+Backspace** cancels. The tray icon (Windows may tuck it under "show hidden icons")
   shows the state and has the menu.

Uninstall from Windows Settings → Apps. Tick "Delete the application data" to also remove the
config, models, history, logs and stored API keys.

Build the installer yourself: `powershell -File scripts/build-release.ps1` (CUDA Toolkit 13.x,
plus the tools in `mise.toml`).

### Commands

| Command | |
| --- | --- |
| `hece run` | the daemon in a console (`hecew` runs it without one) |
| `hece status`, `hece toggle` | ask or drive the running daemon |
| `hece history`, `hece repaste [--raw]` | recent dictations; copy the last one again |
| `hece key set\|delete <provider>` | API keys in the Credential Manager |
| `hece autostart on\|off\|status`, `hece config`, `hece devices` | setup |
| `hece dictate`, `hece bench` | one-off dictation; accuracy and latency benchmarks |

### Known limits

- Opening the microphone takes 0.4–1.1 s on some devices (measured on a Realtek array); the tray
  turns red only when audio actually flows, so start speaking then.
- Pasting into a window running as administrator is blocked by Windows (UIPI) without an error;
  the text stays in `hece history`.
- Transcripts never enter Win+V clipboard history or the cloud clipboard.

### Privacy

- The only network call Hece makes on its own is a once-a-day check of GitHub's latest release (one GET, nothing about you is sent); turn it off with `[updates] check = false` or in the settings.

## Development

See [CONTRIBUTING.md](CONTRIBUTING.md) for prerequisites, the quality gate, and PR expectations.
