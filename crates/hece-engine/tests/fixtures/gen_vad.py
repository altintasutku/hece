"""Generate the VAD parity fixtures. Requires faster-whisper==1.2.1."""
import json
import random
from pathlib import Path

import numpy as np
from faster_whisper import vad
from faster_whisper.utils import get_assets_path
from faster_whisper.vad import SileroVADModel, VadOptions, get_speech_timestamps

out = Path(__file__).resolve().parent
WIN = 512


def markov(seed, n, p_stay_speech, p_stay_silence, start_speech=False):
    """Two-state sequence with speech, silence and ambiguous (0.35-0.5) probabilities."""
    rng = random.Random(seed)
    speech = start_speech
    probs = []
    for _ in range(n):
        stay = p_stay_speech if speech else p_stay_silence
        if rng.random() > stay:
            speech = not speech
        r = rng.random()
        if speech:
            p = rng.uniform(0.55, 1.0) if r > 0.1 else rng.uniform(0.36, 0.49)
        else:
            p = rng.uniform(0.0, 0.3) if r > 0.1 else rng.uniform(0.36, 0.49)
        probs.append(round(p, 4))
    return probs


def periodic(n, speech_run, silence_run, seed):
    """Long speech with short silences (4-6 windows) so the max-speech split finds a pause."""
    rng = random.Random(seed)
    probs = []
    while len(probs) < n:
        probs += [round(rng.uniform(0.6, 1.0), 4) for _ in range(speech_run)]
        probs += [round(rng.uniform(0.0, 0.2), 4) for _ in range(rng.randint(4, silence_run))]
    return probs[:n]


cases = [
    ("default_markov_a", markov(1, 700, 0.99, 0.985), {}),
    ("default_markov_b", markov(2, 900, 0.98, 0.99, True), {}),
    ("short_silence_markov", markov(3, 600, 0.97, 0.95), {"min_silence_duration_ms": 300, "speech_pad_ms": 200}),
    ("silence", [0.01] * 300, {}),
    ("speech", [0.95] * 300, {}),
    ("short_bursts", ([0.9, 0.9] + [0.05] * 90) * 8, {"min_silence_duration_ms": 100}),
    ("max_speech_with_pauses", periodic(800, 40, 6, 4), {"max_speech_duration_s": 4.0, "min_silence_duration_ms": 100}),
    ("max_speech_aggressive", [0.95] * 600, {"max_speech_duration_s": 4.0}),
    ("min_speech_250ms", markov(5, 700, 0.9, 0.96), {"min_speech_duration_ms": 250, "min_silence_duration_ms": 200}),
    ("explicit_neg_threshold", markov(6, 700, 0.97, 0.97), {"threshold": 0.6, "neg_threshold": 0.3, "min_silence_duration_ms": 400}),
]

timestamp_cases = []
rng = random.Random(99)
for name, probs, opts in cases:
    audio_len = (len(probs) - 1) * WIN + rng.randint(0, WIN - 1)
    options = VadOptions(**opts)
    arr = np.array(probs, dtype=np.float32).reshape(-1, 1)
    vad.get_vad_model = lambda arr=arr: (lambda _audio: arr)
    expected = get_speech_timestamps(np.zeros(audio_len, dtype=np.float32), options)
    timestamp_cases.append(
        {
            "name": name,
            "probs": probs,
            "audio_len": audio_len,
            "options": {
                "threshold": options.threshold,
                "neg_threshold": options.neg_threshold,
                "min_speech_duration_ms": options.min_speech_duration_ms,
                "max_speech_duration_s": None if options.max_speech_duration_s == float("inf") else options.max_speech_duration_s,
                "min_silence_duration_ms": options.min_silence_duration_ms,
                "speech_pad_ms": options.speech_pad_ms,
            },
            "expected": [[s["start"], s["end"]] for s in expected],
        }
    )
    print(name, len(expected))
(out / "vad_timestamps.json").write_text(json.dumps(timestamp_cases))

# Synthetic 3 s signal: voiced-like bursts (harmonics with a slow envelope) between low noise.
sr = 16_000
t = np.arange(3 * sr) / sr
noise = np.random.default_rng(7).standard_normal(t.shape[0])
x = 0.004 * noise
for start, end, f0 in [(0.4, 1.2, 140.0), (1.7, 2.5, 190.0)]:
    m = (t >= start) & (t < end)
    env = np.sin(np.pi * (t[m] - start) / (end - start)) ** 0.5 * (0.6 + 0.4 * np.sin(2 * np.pi * 4 * t[m]))
    voiced = sum(np.sin(2 * np.pi * f0 * k * t[m]) / k for k in range(1, 12))
    x[m] += 0.25 * env * voiced / 2 + 0.01 * noise[m]
audio = np.round(np.clip(x, -1, 1) * 32767).astype(np.int16)
pad = WIN - audio.shape[0] % WIN
padded = np.pad(audio.astype(np.float32) / 32768.0, (0, pad))
model = SileroVADModel(str(Path(get_assets_path()) / "silero_vad_v6.onnx"))
probs = model(padded).reshape(-1)
print("probs", probs.shape, float(probs.min()), float(probs.max()))
(out / "vad_probs.json").write_text(json.dumps({"audio": audio.tolist(), "probs": [float(p) for p in probs]}))
