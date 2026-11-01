# Third-party notices

Parts of this crate are ports of, or link, the following MIT-licensed projects.

- faster-whisper 1.2.1, Copyright (c) 2023 SYSTRAN. Ported: log-mel feature extraction
  (`src/mel.rs`) and the decoding, transcription and VAD pipeline logic in the other modules.
- Silero VAD, Copyright (c) 2020-present Silero Team. The speech-probability model usage and
  window/context handling in the VAD module derive from it.
- ONNX Runtime, Copyright (c) Microsoft Corporation. Statically linked through the `ort` crate.
- CTranslate2, Copyright (c) 2018- SYSTRAN / OpenNMT. Linked in `cuda` builds through `ct2rs`.

```
MIT License

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```
