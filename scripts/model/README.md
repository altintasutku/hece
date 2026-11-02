---
license: apache-2.0
language:
  - tr
library_name: ctranslate2
pipeline_tag: automatic-speech-recognition
base_model: turkmedstt/whisper-large-v3-turkish-general
base_model_relation: quantized
tags:
  - whisper
  - ctranslate2
  - faster-whisper
  - turkish
  - hece
---

# whisper-large-v3-turkish-general (CTranslate2, int8)

A [CTranslate2](https://github.com/OpenNMT/CTranslate2) conversion of
[turkmedstt/whisper-large-v3-turkish-general](https://huggingface.co/turkmedstt/whisper-large-v3-turkish-general)
for [Hece](https://github.com/altintasutku/hece), an open-source desktop dictation app. Hece downloads it
with one click; it also works with any CTranslate2 / faster-whisper runtime.

## What changed

Only the format. The weights come from the fine-tuned model at revision
`ec03ccfcedc0cc1c553b47a607086f2bd7a386b2`, converted with `ct2-transformers-converter` (ctranslate2 4.8.3)
using `--quantization int8_float16`. The fine-tuned repo has no `tokenizer.json`, so `tokenizer.json` comes from
[Systran/faster-whisper-large-v3](https://huggingface.co/Systran/faster-whisper-large-v3) (MIT) at revision
`edaa852ec7e145841d8ffdb056a99866b5f0a478`; the conversion checks that the vocabulary matches large-v3 token for
token. The exact, checksummed steps are in Hece's
[`scripts/prepare_model.py`](https://github.com/altintasutku/hece/blob/main/scripts/prepare_model.py).

## Credits and license

- Fine-tuned model: **TurkMedSTT** (Muhammed Kumcu, Yağmur Tuncer), Apache-2.0. See the
  [original model card](https://huggingface.co/turkmedstt/whisper-large-v3-turkish-general) and
  [training data notes](https://github.com/muhammedkumcu/turkmedstt/blob/main/docs/DATA_PIPELINE.md).
- Base model: [OpenAI Whisper large-v3](https://huggingface.co/openai/whisper-large-v3).
- Tokenizer: [Systran/faster-whisper-large-v3](https://huggingface.co/Systran/faster-whisper-large-v3), MIT.

This conversion is distributed under Apache-2.0 (see `LICENSE`). As the original card says, it is not meant for
clinical use.
