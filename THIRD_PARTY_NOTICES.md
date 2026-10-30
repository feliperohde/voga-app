# Third-party notices

Voga includes or uses software from the projects below. Their licenses apply to those portions.

## llama.cpp and llama-cpp-rs

The built-in models run on llama.cpp (https://github.com/ggml-org/llama.cpp) through the llama-cpp-2 and llama-cpp-sys-2 crates (https://github.com/utilityai/llama-cpp-rs, MIT or Apache-2.0).

```text
MIT License

Copyright (c) 2023-2024 The ggml authors

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

## Live sessions: whisper.cpp, ONNX Runtime and the audio crates

Live sessions transcribe with whisper.cpp (https://github.com/ggml-org/whisper.cpp, MIT License, Copyright (c) 2023-2024 The ggml authors) through the whisper-rs crate (https://codeberg.org/tazz4843/whisper-rs, Unlicense), detect speech with the Silero VAD model (https://github.com/snakers4/silero-vad, MIT License) through silero-rs (https://github.com/emotechlab/silero-rs, MIT or Apache-2.0), and compute voice embeddings with pyannote-rs (https://github.com/thewh1teagle/pyannote-rs, MIT License), all on ONNX Runtime (https://github.com/microsoft/onnxruntime, MIT License, Copyright (c) Microsoft Corporation) through the ort crate (https://github.com/pykeio/ort, MIT or Apache-2.0). Sound is captured with cpal (https://github.com/RustAudio/cpal, Apache-2.0) and, on macOS, cidre (https://github.com/yury/cidre, MIT License), and resampled with rubato (https://github.com/HEnquist/rubato, MIT License). The MIT terms are the same as above.

## Models downloaded on demand

Voga does not ship these models: it downloads them when the user asks, and each keeps its own license.

- Qwen 3.5 2B and 4B (Alibaba Qwen team): Apache License 2.0
- Gemma 3 1B and 4B (Google): Gemma Terms of Use, https://ai.google.dev/gemma/terms
- BGE-M3 (BAAI): MIT License
- Nomic Embed Text v1.5 (Nomic AI): Apache License 2.0
- Whisper large-v3-turbo and small (OpenAI), in whisper.cpp's format: MIT License
- WeSpeaker ResNet34 voice embeddings (WeNet community), from sherpa-onnx's releases: Apache License 2.0

## flowsint

Voga installs flowsint (https://github.com/reconurge/flowsint) on the user's computer when asked, from its published container images, with a Docker Compose file adapted from flowsint's `docker-compose.prod.yml`.

```text
Flowsint
Copyright 2025-2026 Reconurge

This product includes software developed by Reconurge.
https://github.com/reconurge/flowsint

Licensed under the Apache License, Version 2.0.
```
