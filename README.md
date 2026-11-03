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

1. Build the zip (`powershell -ExecutionPolicy Bypass -File scripts/package.ps1`; it includes the
   CUDA runtime DLLs, `-NoCuda` leaves them out) or use a release zip, unpack it and run
   `powershell -ExecutionPolicy Bypass -File install.ps1`. It installs to
   `%LOCALAPPDATA%\Programs\Hece` (no administrator rights), adds **Hece** and **Hece Ayarlar** to
   the Start menu and turns on autostart (`-NoAutostart` skips that).
2. Local transcription needs an NVIDIA GPU and a Whisper model. Download one in **Hece Ayarlar**
   or with `hece model download` (`hece model list` shows them): multilingual Whisper large-v3
   turbo by default, or a fine-tuned single-language model such as Turkish. Downloads are pinned
   and checksummed. The CUDA runtime (cuBLAS, cudart; ~426 MB) is downloaded from NVIDIA the same
   way, in **Hece Ayarlar** or with `hece runtime download` (`hece runtime status` checks it); an
   installed CUDA Toolkit 13 on `PATH` is used as is. Without an NVIDIA GPU, use a cloud provider.
3. Open **Hece Ayarlar** and pick the downloaded model, or a cloud provider and its key (stored in the
   Windows Credential Manager). It writes `config.toml` (`hece config` prints the path) and keeps
   your comments. If Hece starts before this is set, it opens the settings window itself.
4. Start Hece from the Start menu. Hold **Ctrl+Shift+Space** and speak; release to insert.
   **Ctrl+Shift+Backspace** cancels. The tray icon (Windows may tuck it under "show hidden icons")
   shows the state and has the menu.

Uninstall: `uninstall.ps1` (`-Purge` also removes config, history and logs).

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

## Development

See [CONTRIBUTING.md](CONTRIBUTING.md) for prerequisites, the quality gate, and PR expectations.
