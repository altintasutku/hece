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

1. Build the zip (`powershell -File scripts/package.ps1`, it includes the CUDA runtime DLLs,
   `-NoCuda` leaves them out) or use a release zip, unpack it and run
   `powershell -ExecutionPolicy Bypass -File install.ps1`. It installs to
   `%LOCALAPPDATA%\Programs\Hece` (no administrator rights), adds a Start menu entry and turns on
   autostart (`-NoAutostart` skips that).
2. Local transcription needs an NVIDIA GPU and the Whisper model:
   `uv run prepare_model.py <dir>` (install.ps1 prints the suggested `<dir>`) converts the
   Turkish fine-tuned large-v3 (`turkmedstt/whisper-large-v3-turkish-general`, pinned and
   checksummed).
   Without the CUDA runtime, use a cloud provider instead.
3. `hece config` prints the config path (a commented template is created on first run). Set
   `[provider] model_dir`, or a cloud provider and its key (`hece key set <name>` stores it in the
   Windows Credential Manager).
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
