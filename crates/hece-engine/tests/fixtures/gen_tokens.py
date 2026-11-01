"""Generate the tokenizer parity fixture. Requires faster-whisper==1.2.1.

Usage: python gen_tokens.py <model_dir containing tokenizer.json>
"""
import json
import sys
from pathlib import Path

import tokenizers
from faster_whisper.tokenizer import Tokenizer
from faster_whisper.transcribe import get_suppressed_tokens

model_dir = Path(sys.argv[1])
raw = tokenizers.Tokenizer.from_file(str(model_dir / "tokenizer.json"))
tok = Tokenizer(raw, multilingual=True, task="transcribe", language="tr")

text = " Merhaba dünya, Kubernetes ve PostgreSQL."
expected = {
    "sot": tok.sot,
    "sot_prev": tok.sot_prev,
    "sot_lm": tok.sot_lm,
    "eot": tok.eot,
    "transcribe": tok.transcribe,
    "translate": tok.translate,
    "no_timestamps": tok.no_timestamps,
    "no_speech": tok.no_speech,
    "timestamp_begin": tok.timestamp_begin,
    "language_tr": tok.language,
    "sot_sequence": list(tok.sot_sequence),
    "suppressed": list(get_suppressed_tokens(tok, [-1])),
    "text": text,
    "text_tokens": tok.encode(text),
}
out = Path(__file__).resolve().parent / "tokens_expected.json"
out.write_text(json.dumps(expected, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
