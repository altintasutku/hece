"""Generate the mel parity fixture. Requires faster-whisper==1.2.1."""
from pathlib import Path

import numpy as np
from faster_whisper.feature_extractor import FeatureExtractor

out = Path(__file__).resolve().parent
sr = 16_000
t = np.arange(int(1.5 * sr)) / sr
noise = np.random.default_rng(1234).standard_normal(t.shape[0])
x = 0.3 * np.sin(2 * np.pi * 440 * t) + 0.2 * np.sin(2 * np.pi * 1234.5 * t) + 0.05 * noise
x = np.clip(x, -1.0, 1.0)
audio = np.round(x * 32767).astype("<i2")
audio.tofile(out / "mel_audio.i16")

mel = FeatureExtractor(feature_size=128)(audio.astype(np.float32) / 32768.0, padding=160)
mel.astype("<f4").tofile(out / "mel_expected.f32")
print(mel.shape)
