# Test fixtures

All fixtures are synthetic (generated tones and noise).

| File | Generator | Description |
| --- | --- | --- |
| `mel_audio.i16` | `gen_mel.py` | 1.5 s of 16 kHz mono PCM, little-endian i16 |
| `mel_expected.f32` | `gen_mel.py` | faster-whisper log-mel of that audio, 128 x frames, row-major little-endian f32 |
| `vad_timestamps.json` | `gen_vad.py` | Seeded probability sequences, VAD options and faster-whisper's `get_speech_timestamps` output |
| `vad_probs.json` | `gen_vad.py` | A 3 s synthetic i16 signal and faster-whisper's Silero probabilities for it |

Generator scripts require `faster-whisper==1.2.1`.
