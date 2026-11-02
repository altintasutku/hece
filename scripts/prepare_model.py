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

Pinned to one commit, and every file used is checked against a sha256 recorded here. The
fine-tuned repo has no tokenizer.json, so it comes from faster-whisper-large-v3 at a pinned
revision; the converted vocabulary must equal large-v3's token for token, otherwise that
tokenizer would be wrong.
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
# Every file the converter reads; nothing else in the repo is downloaded.
FILES = {
    "model.safetensors": "1942821b3742a39b1e7e9b68d4beaef0b54c3bed9a6bd94cc7850816d07f00c0",
    "config.json": "bf436e80bbc02c820a2176e7f722f482009c50995774364818a1d72006aa5800",
    "generation_config.json": "d3b30a459f7d93eaba9006c16c962cc18d41435ad7ea19e0b13c8d8e7acba107",
    "preprocessor_config.json": "7ccc62c6f2765af1f3b46c00c9b5894426835a05021c8b9c01eecb6dfb542711",
    "added_tokens.json": "3c51f66c4c21f9e126970078f11ae77a78c74aee8df606ee9daba86e467108e0",
    "merges.txt": "2df2990a395e35e8dfbc7511e08c12d56018d8d04691e0133e5d63b21e154dc6",
    "normalizer.json": "bf1c507dc8724ca9cf9903640dacfb69dae2f00edee4f21ceba106a7392f26dd",
    "special_tokens_map.json": "baea4ea09372eb4fca86b4e4346139fd73cb807d5087e9de0948e971739c3e74",
    "tokenizer_config.json": "844b642c73a91359722f47b35705f7174686df33d252695d8572cf9ac03a6389",
    "vocab.json": "e2aa043ef015641d363d8288e7c241c85e36a5c761fb303598e0710233344387",
}
BASE_REPO = "Systran/faster-whisper-large-v3"
BASE_REVISION = "edaa852ec7e145841d8ffdb056a99866b5f0a478"
BASE_FILES = {
    "vocabulary.json": "c69260f2ab26d659b7c398f9a2b2b48ed0df16c3b47d7326782fd9cba71690c1",
    "tokenizer.json": "6d8cbd7cd0d8d5815e478dac67b85a26bbe77c1f5e0c6d76d1ce2abc0e5f21ca",
}
DEFAULT_OUT = Path(".cache/models") / f"large-v3-turkish-{REVISION[:7]}-int8"


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
    src = Path(snapshot_download(REPO, revision=REVISION, allow_patterns=list(FILES)))
    base = {name: Path(hf_hub_download(BASE_REPO, name, revision=BASE_REVISION)) for name in BASE_FILES}
    expected = [(src / name, digest) for name, digest in FILES.items()]
    expected += [(base[name], digest) for name, digest in BASE_FILES.items()]
    for path, digest in expected:
        if sha256(path) != digest:
            print(f"sha256 mismatch: {path.name}", file=sys.stderr)
            return 1
    tmp = out.with_name(out.name + ".tmp")
    shutil.rmtree(tmp, ignore_errors=True)
    converter = Path(sys.executable).parent / (
        "ct2-transformers-converter.exe" if sys.platform == "win32" else "ct2-transformers-converter"
    )
    subprocess.run(
        [str(converter), "--model", str(src), "--output_dir", str(tmp),
         "--quantization", "int8_float16", "--copy_files", "preprocessor_config.json",
         "--low_cpu_mem_usage"],
        check=True,
    )
    tokens = lambda p: json.loads(p.read_text(encoding="utf-8"))  # noqa: E731
    if tokens(tmp / "vocabulary.json") != tokens(base["vocabulary.json"]):
        print("vocabulary differs from large-v3; tokenizer.json would be wrong", file=sys.stderr)
        return 1
    shutil.copyfile(base["tokenizer.json"], tmp / "tokenizer.json")
    tmp.rename(out)
    print(f"prepared: {out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
