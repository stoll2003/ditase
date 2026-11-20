# Dita.se para Windows: casca

App Tauri 2 (WebView2) que serve a mesma interface do Mac (`ui/`) e fala com ela pelo mesmo contrato da ponte (`docs/contrato-ponte-lancamento.md`). O transporte muda só no adaptador de `ui/src/lib/ponte.ts`: no Tauri vai por `invoke("ponte", {metodo, params})`; os avisos voltam pelo mesmo `CustomEvent` do Mac.

## Peças

| Arquivo | O quê | Porta de |
|---|---|---|
| `src-tauri/src/main.rs` | janela principal, bandeja, instância única, iniciar com o Windows, arraste, atualização do canal de teste | `JanelaPrincipal`, `main.swift` |
| `src-tauri/src/ponte.rs` | os métodos do contrato (`METODOS`, com situação de cada um) | `Ponte.swift` |
| `src-tauri/src/controlador.rs` | ciclo do ditado | `Controlador.swift` |
| `src-tauri/src/atalho.rs` | gestos do atalho (segurar, dois toques, Espaço, Esc, Ctrl+Alt+V), testável no Mac | `Atalho.swift` |
| `src-tauri/src/nativo/win_gancho.rs` | gancho `WH_KEYBOARD_LL`: só a tecla escolhida, Esc, Espaço e Ctrl+Alt+V; nada é guardado | event tap |
| `src-tauri/src/nativo/win_colagem.rs` | área de transferência + `SendInput` Ctrl+V, restaura o conteúdo; app como administrador fica só copiado | `Insersor.swift` |
| `src-tauri/src/audio.rs` | microfone e áudio do sistema (loopback WASAPI) via `cpal`, 16 kHz mono, WAV | `Gravador`, `CapturaReuniao` |
| `src-tauri/src/decodificacao.rs`, `nativo/win_midia.rs` | Media Foundation (mp4, mov, m4a) e Symphonia (wav, flac, mp3) | `AudioLancamento` |
| `src-tauri/src/pilula.rs`, `pilula/pilula.html` | pílula: janela transparente, no topo, sem roubar foco | `Pilula`, `PilulaProgresso` |
| `src-tauri/src/preferencias.rs`, `caminhos.rs` | ajustes em JSON em `%APPDATA%\Dita.se`; áudio e modelos em `%LOCALAPPDATA%\Dita.se` | `Preferencias`, `Caminhos` |
| `src-tauri/src/integracao.rs` | ditado e histórico sobre `nucleo` (Banco, SessaoDitado, dicionário, estatísticas, recuperação de interrompidos); trait `Motores` e dublês de teste | `Controlador` |
| `src-tauri/src/lancamento.rs` | arquivos e reuniões: fila com `ProcessadorDeArquivo`, captura em `ColetorDeReuniao`, `ProcessadorDeReuniao` ao vivo, passada final de falantes, recuperação, resumo | `ServicoLancamento` |
| `src-tauri/src/motores_locais.rs` | Parakeet, diarização e llama-server; download dos modelos com retomada, SHA-256 e cancelamento | `TranscritorParakeet`, `ServidorLLM`, `ModeloIA` |
| `src-tauri/src/relatorio.rs` | "Copiar relatório de teste" (bandeja e método `relatorio.copiar`) | |
| `src-tauri/src/teste.rs` | modos de teste por linha de comando | |

Atalho padrão: Ctrl direito (no ABNT2 o Alt direito é AltGr). Ids aceitos: `controleDireito`, `shiftDireito`, `altDireito`.

## Compilar

```sh
cd ui && npm ci && npm run build        # o build.rs monta windows/app-dist com ui/dist + pilula.html
cargo check -p ditase                    # no Mac: substitutos inertes no lugar do Win32
cargo test -p ditase
```

No Windows (CI): `cargo tauri build --config windows/src-tauri/tauri.distribuicao.conf.json`, com `windows/updater.pub` (chave pública do canal de teste) e a pasta `windows/vendor/` preenchida:

```
windows/vendor/sherpa/      sherpa-onnx-c-api.dll, onnxruntime.dll, onnxruntime_providers_shared.dll (copiadas de target\release pelo build do crate sherpa-onnx)
windows/vendor/llama/       llama-server.exe e as DLLs do llama.cpp b12359 CPU, libomp.dll, LICENSE-LLVM-OpenMP (lista em motores/LICENCAS.md)
windows/vendor/vcruntime/   MSVCP140.dll, VCRUNTIME140.dll, VCRUNTIME140_1.dll (Redist do Visual Studio do runner); vão ao lado do .exe e em llama\
windows/vendor/avisos/      ThirdPartyNotices.txt do ONNX Runtime
```

O manifesto (`src-tauri/app.manifest`) declara `activeCodePage` UTF-8, pedido dos motores para pastas de usuário com acento.

Para o `cargo check --target x86_64-pc-windows-msvc` no Mac, o `tauri-winres` exige um `llvm-rc`; um substituto que só cria o `.res` vazio basta para checar (não linka).

## Modos de teste (sessão no PC)

```
Dita.se.exe --testar-captura 10              10 s de microfone e de áudio do sistema; WAVs em %LOCALAPPDATA%\Dita.se\ReunioesAudio
Dita.se.exe --testar-arquivo C:\video.mp4    decodifica e mede
```

O resultado (só números) vai para `%APPDATA%\Dita.se\teste-*.json` e para o registro.

## Modelos

- Fala (Parakeet) e falantes (pyannote + CAM++), cerca de 706 MB: baixam sozinhos na primeira abertura; o progresso aparece no estado do reconhecedor ("Baixando o modelo de fala: N%"). Até terminar, o ditado avisa na pílula e Arquivos/Reunião ficam indisponíveis.
- IA local (Qwen, 1,28 GB): só pelo botão do Início, com progresso e cancelamento. Ao terminar, a limpeza liga.
- Limpeza por IA desligada por padrão no Windows (llama.cpp de CPU). Se o servidor não subir em 60 s ou falhar, ela desliga sozinha e o ditado segue com regras e dicionário.
- Arquivos em `%LOCALAPPDATA%\Dita.se\Models`; todo download confere o SHA-256 antes de renomear.
