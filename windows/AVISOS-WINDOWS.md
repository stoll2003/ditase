# Avisos de terceiros do Dita.se para Windows

Complementa o `THIRD-PARTY-NOTICES.md` do projeto. Origem e revisão de cada peça: `motores/LICENCAS.md`.

## Programas que vão no instalador

- **sherpa-onnx** 2.0.1 (k2-fsa), `sherpa-onnx-c-api.dll`: Apache License 2.0.
- **ONNX Runtime** (Microsoft), `onnxruntime.dll` e `onnxruntime_providers_shared.dll`: MIT. Os avisos de terceiros do ONNX Runtime estão em `avisos/ThirdPartyNotices.txt`.
- **llama.cpp e ggml** b12359 (ggml-org), pasta `llama\`: MIT. Inclui `cpp-httplib`, `nlohmann/json` e `minja` (MIT).
- **LLVM OpenMP**, `llama\libomp.dll`: Apache License 2.0 com exceção LLVM (`llama\LICENSE-LLVM-OpenMP`).
- **Microsoft Visual C++ Runtime** (`MSVCP140.dll`, `VCRUNTIME140.dll`, `VCRUNTIME140_1.dll`): redistribuído conforme a lista "Redist" dos termos do Visual Studio.

## Modelos baixados pelo app

- **Parakeet TDT 0.6B v3** (NVIDIA), licença CC-BY-4.0 (https://creativecommons.org/licenses/by/4.0/). Modificação: convertido para ONNX e quantizado em INT8 pelo projeto sherpa-onnx; usado sem outras alterações.
- **pyannote segmentation-3.0** (CNRS), MIT, Copyright (c) 2022 CNRS. Exportado para ONNX pelo projeto sherpa-onnx.
- **CAM++** do 3D-Speaker (Alibaba, ModelScope `iic/speech_campplus_sv_en_voxceleb_16k`): Apache License 2.0.
- **Qwen3.5 2B** (Qwen), GGUF da Unsloth: Apache License 2.0.
