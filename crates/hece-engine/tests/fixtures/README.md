# Test fixtures

All fixtures are synthetic (generated tones and noise).

| File | Generator | Description |
| --- | --- | --- |
| `mel_audio.i16` | `gen_mel.py` | 1.5 s of 16 kHz mono PCM, little-endian i16 |
| `mel_expected.f32` | `gen_mel.py` | faster-whisper log-mel of that audio, 128 x frames, row-major little-endian f32 |

Generator scripts require `faster-whisper==1.2.1`.
