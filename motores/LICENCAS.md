# Motores do Dita.se para Windows: licenças, arquivos e dependências

Rastreio de cada componente até a origem. Revisão: 19/11/2026.

## 1. Modelos baixados pelo app (não vão no instalador)

Baixados na primeira execução, de URL fixada em revisão, com tamanho e SHA-256 conferidos (`src/catalogo.rs`). Total de fala e falantes: cerca de 706 MB; com o Qwen, cerca de 1,99 GB.

| Conjunto | Arquivo | Tamanho | Origem do peso | Licença | Redistribuição |
|---|---|---|---|---|---|
| Fala | `parakeet/encoder.int8.onnx`, `decoder.int8.onnx`, `joiner.int8.onnx`, `tokens.txt` | 670,6 MB | `nvidia/parakeet-tdt-0.6b-v3`, exportado para ONNX INT8 pelo sherpa-onnx (`csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8`, revisão `ed5d7456`) | CC-BY-4.0 | Permitida com atribuição. O app deve creditar a NVIDIA, citar a licença e indicar a modificação (conversão para ONNX e quantização INT8) na tela de licenças/avisos |
| Falantes, segmentação | `falantes/segmentation.onnx` | 6,0 MB | `pyannote/segmentation-3.0` (CNRS), exportado pelo sherpa-onnx (`csukuangfj/sherpa-onnx-pyannote-segmentation-3-0`, revisão `9403a690`, com o `LICENSE` original) | MIT (Copyright 2022 CNRS) | Permitida. O repositório original no Hugging Face pede cadastro para baixar ("gated"), mas isso não é cláusula da licença MIT; a cópia convertida já é redistribuída com a licença. Os dados de treino incluem conjuntos de licença restrita (por exemplo DIHARD/LDC); os autores publicaram os pesos sob MIT, que é o que vale para nós. Risco residual baixo |
| Falantes, embeddings | `falantes/3dspeaker_speech_campplus_sv_en_voxceleb_16k.onnx` | 29,6 MB | CAM++ do 3D-Speaker (Alibaba/ModelScope `iic/speech_campplus_sv_en_voxceleb_16k`), espelho `csukuangfj/speaker-embedding-models`, revisão `a8055d6d` | Apache-2.0 (cartão do modelo no ModelScope e repositório 3D-Speaker) | Permitida, com aviso de licença. Treinado em VoxCeleb (dados CC-BY-4.0) |
| IA de limpeza | `Qwen3.5-2B-Q4_K_M.gguf` | 1,28 GB | `Qwen/Qwen3.5-2B`, GGUF da Unsloth (mesmo do Mac) | Apache-2.0 | Já documentado no `THIRD-PARTY-NOTICES.md` |

Escolhas descartadas para embeddings: NeMo TitaNet (CC-BY-4.0, maior) e WeSpeaker VoxCeleb (licença dos pesos menos clara). CAM++ VoxCeleb tem Apache-2.0 confirmada na origem e é pequeno.

## 2. O que o instalador Windows precisa levar (pasta do `.exe`)

Tudo ao lado do executável do app, para o Windows achar as DLLs pela pasta do aplicativo antes do `System32`.

### sherpa-onnx (fala e falantes)
Do `sherpa-onnx-v2.0.1-win-x64-shared-MT-Release-lib.tar.bz2` (release oficial k2-fsa; o build do crate baixa e copia para `target\release` sozinho; SHA-256 `afa4de3ca4ab0afd53bb4aab40d28ee15608364b549ffc0fe0caf6489bfc39a5`):

- `sherpa-onnx-c-api.dll` (4,3 MB): Apache-2.0
- `onnxruntime.dll` (17,8 MB): MIT (Microsoft). Levar também o `ThirdPartyNotices.txt` do ONNX Runtime
- `onnxruntime_providers_shared.dll` (0,1 MB): MIT. Só é usada por provedores extras; vai junto por segurança

Compiladas com CRT estático (/MT): importam só `KERNEL32`, `ADVAPI32`, `SETUPAPI`, `dbghelp`, `dxgi` e `api-ms-win-core-path`, todas do Windows. Não precisam do Visual C++ Redistributable.

Atenção: o Windows 11 tem um `onnxruntime.dll` próprio (Windows ML) em `System32`, de outra versão. Com a DLL ao lado do `.exe` (carga implícita) a do app é a usada. Não mover as DLLs para subpasta nem carregar por `LoadLibrary` com caminho relativo.

### llama.cpp (limpeza e resumo)
Do `llama-b12359-bin-win-cpu-x64.zip` (release oficial ggml-org, mesma versão `b12359` do Mac; SHA-256 `837c1655e87df3b66989f4be33a792a1399c785bf259038dc75c900096683e69`), numa subpasta `llama\`:

- `llama-server.exe`, `llama-server-impl.dll`, `llama-common.dll`, `llama.dll`, `mtmd.dll`
- `ggml.dll`, `ggml-base.dll`, `ggml-rpc.dll`, todos os `ggml-cpu-*.dll` (13 variantes; o ggml escolhe a melhor para a CPU na hora)
- `libomp.dll` e `LICENSE-LLVM-OpenMP` (LLVM OpenMP, Apache-2.0 com exceção LLVM)

Total cerca de 45 MB. Licença MIT (llama.cpp e ggml), já no `THIRD-PARTY-NOTICES.md`; o build do Windows também incorpora `cpp-httplib`, `nlohmann/json` e `minja` (MIT).

**Diferente das DLLs do sherpa-onnx, essas importam `MSVCP140.dll`, `VCRUNTIME140.dll` e `VCRUNTIME140_1.dll`** (Visual C++ 2015-2022). Windows 11 limpo nem sempre tem. O instalador deve levar essas três DLLs na pasta `llama\` (são redistribuíveis pela lista "Redist" da Microsoft, vindas do Visual Studio do runner de CI) ou encadear o `VC_redist.x64.exe`. Sem isso o `llama-server.exe` não abre e a limpeza fica indisponível (o app segue com regras e dicionário).

### Avisos de licença a acrescentar ao `THIRD-PARTY-NOTICES.md` do Windows
sherpa-onnx (Apache-2.0), ONNX Runtime (MIT + ThirdPartyNotices), LLVM OpenMP, e os modelos da seção 1 (Parakeet com a atribuição CC-BY-4.0, pyannote MIT, CAM++ Apache-2.0).

## 3. Dependências Rust do crate `motores`

| Crate | Versão | Licença | Para quê |
|---|---|---|---|
| `sherpa-onnx`, `sherpa-onnx-sys` | =2.0.1 | Apache-2.0 | API oficial do sherpa-onnx (Mac: estático; Windows: DLLs) |
| `ureq` | 2 | MIT/Apache-2.0 | HTTP para o llama-server (127.0.0.1) e download |
| `rustls` (Mac/Linux) ou `native-tls` (Windows, SChannel) | via ureq | MIT/Apache-2.0 | HTTPS do download |
| `sha2`, `serde_json` | 0.10, 1 | MIT/Apache-2.0 | hash e JSON |
| `windows-sys` (Windows) | 0.59 | MIT/Apache-2.0 | Job Object, nome curto de caminho, memória |
| `libc` (Unix) | 0.2 | MIT/Apache-2.0 | sinal e memória |

## 4. Notas para a casca Windows (`windows/`)

- Declarar `<activeCodePage>UTF-8</activeCodePage>` no manifesto do app. A API C do sherpa-onnx e o `llama-server` abrem arquivos com `char*`: sem isso, pasta de usuário com acento (`C:\Users\João\AppData\...`) quebra a carga dos modelos. O `motores` já tenta o nome curto 8.3 como segunda via (`audio::caminho_c`), mas ele pode estar desligado no volume.
- O `llama-server.exe` sobe sem janela de console (`CREATE_NO_WINDOW`) e dentro de um Job Object com "matar ao fechar": se o app cair, o Windows encerra o servidor. Não há arquivo de PID como no Mac.
- No Windows o encerramento é direto (`TerminateProcess`), sem o pedido educado de 2 s do Mac: o servidor não guarda estado.
