# /// script
# requires-python = ">=3.12"
# dependencies = [
#   "huggingface-hub==2.2.0",
# ]
# ///
"""Publishes a model made by prepare_model.py to a Hugging Face repo, with its card and license.

    HF_TOKEN=hf_... uv run scripts/publish_model.py <model_dir> <owner/repo>

Prints the commit and each file's size and sha256, ready for Hece's model catalog. The token is
read from HF_TOKEN only.
"""

import hashlib
import json
import os
import shutil
import sys
import tempfile
import urllib.request
from pathlib import Path

from huggingface_hub import HfApi

FILES = ["model.bin", "config.json", "tokenizer.json", "vocabulary.json", "preprocessor_config.json"]
CARD = Path(__file__).parent / "model" / "README.md"
LICENSE_URL = "https://www.apache.org/licenses/LICENSE-2.0.txt"


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def main() -> None:
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    model_dir, repo = Path(sys.argv[1]), sys.argv[2]
    token = os.environ.get("HF_TOKEN")
    if not token:
        sys.exit("HF_TOKEN is not set")
    missing = [f for f in FILES if not (model_dir / f).is_file()]
    if missing:
        sys.exit(f"missing in {model_dir}: {', '.join(missing)}")
    with tempfile.TemporaryDirectory() as tmp:
        stage = Path(tmp)
        for name in FILES:
            # Hard links keep 1.5 GB from being copied when the volume allows it.
            try:
                os.link(model_dir / name, stage / name)
            except OSError:
                shutil.copy2(model_dir / name, stage / name)
        shutil.copy2(CARD, stage / "README.md")
        with urllib.request.urlopen(LICENSE_URL) as response:
            (stage / "LICENSE").write_bytes(response.read())
        api = HfApi(token=token)
        api.create_repo(repo, repo_type="model", exist_ok=True)
        commit = api.upload_folder(
            repo_id=repo,
            folder_path=stage,
            commit_message="Upload CTranslate2 int8 conversion",
        )
    manifest = {
        "repo": repo,
        "revision": commit.oid,
        "files": [
            {"name": f, "size": (model_dir / f).stat().st_size, "sha256": sha256(model_dir / f)}
            for f in FILES
        ],
    }
    print(json.dumps(manifest, indent=2))


if __name__ == "__main__":
    main()
