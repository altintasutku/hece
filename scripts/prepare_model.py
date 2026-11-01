# /// script
# requires-python = ">=3.12"
# dependencies = [
#   "ctranslate2==4.8.3",
#   "transformers==5.20.1",
#   "torch==2.14.1",
#   "huggingface-hub==2.2.0",
# ]
# [[tool.uv.index]]
# name = "pytorch-cpu"
# url = "https://download.pytorch.org/whl/cpu"
# explicit = true
# [tool.uv.sources]
# torch = { index = "pytorch-cpu" }
# ///
"""Converts the Turkish fine-tuned Whisper model to CTranslate2 for hece-engine.

    uv run scripts/prepare_model.py [output_dir]

Pinned to one commit and checked by the weights' sha256. The fine-tuned repo has no
tokenizer.json, so it comes from faster-whisper-large-v3 at a pinned revision; the converted
vocabulary must equal large-v3's token for token, otherwise that tokenizer would be wrong.
"""

import hashlib
import json
import shutil
import subprocess
import sys
from pathlib import Path

from huggingface_hub import hf_hub_download, snapshot_download

REPO = "turkmedstt/whisper-large-v3-turkish-general"
REVISION = "ec03ccfcedc0cc1c553b47a607086f2bd7a386b2"
WEIGHTS_SHA256 = "1942821b3742a39b1e7e9b68d4beaef0b54c3bed9a6bd94cc7850816d07f00c0"
BASE_REPO = "Systran/faster-whisper-large-v3"
BASE_REVISION = "edaa852ec7e145841d8ffdb056a99866b5f0a478"
DEFAULT_OUT = Path(".cache/models") / f"large-v3-turkish-{REVISION[:7]}"


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as f:
        while chunk := f.read(8 * 1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def main() -> int:
    out = Path(sys.argv[1]) if len(sys.argv) > 1 else DEFAULT_OUT
    if (out / "model.bin").is_file():
        print(f"already prepared: {out}")
        return 0
    src = Path(snapshot_download(REPO, revision=REVISION))
    if sha256(src / "model.safetensors") != WEIGHTS_SHA256:
        print("weights sha256 mismatch", file=sys.stderr)
        return 1
    tmp = out.with_name(out.name + ".tmp")
    shutil.rmtree(tmp, ignore_errors=True)
    converter = Path(sys.executable).parent / (
        "ct2-transformers-converter.exe" if sys.platform == "win32" else "ct2-transformers-converter"
    )
    subprocess.run(
        [str(converter), "--model", str(src), "--output_dir", str(tmp),
         "--quantization", "float16", "--copy_files", "preprocessor_config.json",
         "--low_cpu_mem_usage"],
        check=True,
    )
    base = lambda name: Path(hf_hub_download(BASE_REPO, name, revision=BASE_REVISION))  # noqa: E731
    tokens = lambda p: json.loads(p.read_text(encoding="utf-8"))  # noqa: E731
    if tokens(tmp / "vocabulary.json") != tokens(base("vocabulary.json")):
        print("vocabulary differs from large-v3; tokenizer.json would be wrong", file=sys.stderr)
        return 1
    shutil.copyfile(base("tokenizer.json"), tmp / "tokenizer.json")
    tmp.rename(out)
    print(f"prepared: {out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
