# Hece

Background voice dictation for the desktop. Press a hotkey, speak, and the text lands in the focused window.

- Toggle and push-to-talk modes
- Switchable speech-to-text providers: local Whisper (CTranslate2, CUDA), OpenAI-compatible endpoints, Deepgram
- Quality layers: personal vocabulary, hallucination filter, low-confidence signal, optional guarded LLM cleanup
- Tiny when idle; the local model loads on demand and is released when idle
- Local OpenAI-compatible API so other apps can share one loaded model

Status: early development. Windows first; Linux and macOS later.

License: MIT

## Development

See [CONTRIBUTING.md](CONTRIBUTING.md) for prerequisites, the quality gate, and PR expectations.
