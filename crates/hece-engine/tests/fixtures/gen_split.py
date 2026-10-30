"""Generate the timestamp-split and compression-ratio parity fixture. Requires faster-whisper==1.2.1.

Usage: python gen_split.py
"""
import json
from pathlib import Path
from types import SimpleNamespace

from faster_whisper.transcribe import WhisperModel, get_compression_ratio

here = Path(__file__).resolve().parent
special = json.loads((here / "tokens_expected.json").read_text(encoding="utf-8"))
tb = special["timestamp_begin"]
eot = special["eot"]

# Only the attributes `_split_segments_by_timestamps` reads (see `WhisperModel.__init__`).
model = object.__new__(WhisperModel)
model.input_stride = 2
model.time_precision = 0.02
model.frames_per_second = 100
tokenizer = SimpleNamespace(timestamp_begin=tb, eot=eot)


def t(n):
    return tb + n


cases = {
    "two_segments_single_ending": ([t(0), 100, 101, t(50), t(50), 102, 103, t(120)], 0),
    "trailing_text_without_closing": ([t(0), 100, t(40), t(40), 101, t(90), t(90), 102], 0),
    "consecutive_at_end": ([t(0), 100, t(50), t(50), 101, t(80), t(80)], 0),
    "single_timestamp_ending": ([t(0), 100, 101, t(75)], 0),
    "no_timestamps": ([100, 101, 102], 0),
    "only_begin_timestamp": ([100, t(0)], 0),
    "short_segment_offset": ([t(0), 100, t(30), t(30), 101, t(55)], 3000),
    "short_segment_trailing": ([t(2), 100, 101, t(25), t(25), 102], 3000),
}
segment_sizes = {"short_segment_offset": 1200, "short_segment_trailing": 1200}

out_cases = []
for name, (tokens, seek) in cases.items():
    segment_size = segment_sizes.get(name, 3000)
    time_offset = seek * 0.01
    segment_duration = segment_size * 0.01
    segments, next_seek, single = model._split_segments_by_timestamps(
        tokenizer, tokens, time_offset, segment_size, segment_duration, seek
    )
    out_cases.append(
        {
            "name": name,
            "tokens": tokens,
            "seek": seek,
            "time_offset": time_offset,
            "segment_size": segment_size,
            "segment_duration": segment_duration,
            "segments": [
                {"start": s["start"], "end": s["end"], "tokens": list(s["tokens"])}
                for s in segments
            ],
            "next_seek": next_seek,
            "single_timestamp_ending": single,
        }
    )

texts = [
    "hello hello hello hello",
    "a" * 100,
    "The quick brown fox jumps over the lazy dog.",
    "Bugün hava çok güzel, dışarı çıkalım.",
    "",
]
compression = [
    {"text": s, "ratio": get_compression_ratio(s)} for s in texts
]

expected = {"timestamp_begin": tb, "eot": eot, "cases": out_cases, "compression": compression}
(here / "split_expected.json").write_text(
    json.dumps(expected, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
)
