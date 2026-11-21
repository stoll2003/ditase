# Dita.se

Dite em qualquer app e transcreva reuniões ou arquivos de áudio e vídeo com separação de falantes. O áudio e o texto ficam no seu computador.

## Baixar

- [Baixar para Mac](https://github.com/stoll2003/ditase/releases/latest/download/Dita.se-mac-arm64.dmg) · macOS 15 ou mais recente, com chip Apple (M1 ou superior).
- [Baixar para Windows](https://github.com/stoll2003/ditase/releases/latest/download/Dita.se-windows-x64-setup.exe) · Windows 10 (1903 ou mais recente) ou 11, x64.

Veja as [notas da versão mais recente](https://github.com/stoll2003/ditase/releases/latest) para conferir o download ou resolver avisos na primeira abertura.

## Telas

![Tela de início do Dita.se](docs/screens-publicas/inicio-claro.png)

[Reunião](docs/screens-publicas/reuniao-claro.png) · [Arquivos](docs/screens-publicas/arquivos-claro.png) · [Ajustes](docs/screens-publicas/ajustes-claro.png)

<details>
<summary><strong>Instalação, permissões e desinstalação</strong></summary>

### Mac

Abra o DMG e arraste o Dita.se para **Aplicativos**. Se o macOS bloquear a primeira abertura, tente abrir o app, feche o aviso e vá a **Ajustes do Sistema** > **Privacidade e Segurança** > **Abrir Mesmo Assim**. Para ditar, segure **Opção direita**.

O macOS pede acesso ao **Microfone** para ouvir a sua voz, à **Acessibilidade** para acionar o atalho e colar e à gravação de áudio do sistema para capturar a chamada. O app não grava imagens da tela.

Para instalar pelo Terminal, rode:

```bash
curl -fsSL -H 'Accept: application/vnd.github.raw+json' 'https://api.github.com/repositories/1464733163/contents/scripts/install.sh?ref=main' | bash
```

Para desinstalar, mova o app para a Lixeira. Os dados e o modelo de limpeza ficam em `~/Library/Application Support/Ditado`. Os modelos de fala e falantes ficam no cache compartilhado `~/Library/Application Support/FluidAudio/Models`.

### Windows

Abra o instalador. Ele instala o app para sua conta, sem senha de administrador. Para ditar, segure **Ctrl direito**. Você pode trocar a tecla em **Ajustes**. No teclado brasileiro, o Alt direito funciona como AltGr.

Na primeira abertura, o app baixa cerca de 700 MB em modelos de fala e de separação de falantes. A limpeza com IA é opcional e pode ser baixada pela aba **Início**. O app consulta atualizações ao abrir e pede confirmação antes de instalar.

Para desinstalar, vá a **Configurações** > **Aplicativos**. Os dados e modelos ficam em `%LOCALAPPDATA%\Dita.se`.

### Privacidade

O reconhecimento de fala, a separação de vozes, a limpeza e os resumos rodam no computador. O app usa a internet para baixar modelos e, no Windows, para consultar e baixar atualizações.

</details>

<details>
<summary><strong>Para desenvolvedores</strong></summary>

### Arquitetura do projeto

#### Versão Mac

- **`Nucleo/` (pacote Swift independente de interface):** segmentador de áudio por pausas, sessão de ditado, regras determinísticas de fala (`RegrasFala`), dicionário fonético (`AplicadorDicionario`), travas contra alucinação ou reescrita do modelo (`Guarda` e `Fidelidade`), persistência transacional de ditados, arquivos e reuniões em SQLite (`Banco`, `ArmazemLancamento`, `Costura`) e cálculo de estatísticas.
- **`Ditado/` (aplicativo macOS em AppKit + SwiftUI):** captura de atalho por event tap, gravação de microfone e de áudio do sistema (ScreenCaptureKit), decodificação de mídia (AVFoundation), integração com FluidAudio (Parakeet Ultra e Sortformer), gerenciamento sob demanda do processo `llama-server`, pílula flutuante e ponte WebKit (`ditado://`).
- **`ui/` (React 19 + TypeScript + Vite + Tailwind CSS 4 + Tuff UI):** interface da janela principal com as abas Início, Reunião, Arquivos, Insights, Dicionário e Ajustes.
- **`project.yml`:** especificação do XcodeGen que gera `Ditado.xcodeproj`.

#### Versão Windows

- **`windows/src-tauri/`** usa Tauri 2 e WebView2 para exibir a mesma interface de `ui/`. Implementa o gancho de teclado, a colagem sem registrar o texto temporário no histórico do Win+V, a captura WASAPI de microfone e áudio do sistema, a pílula e a bandeja.
- **`nucleo-rs/`** porta o núcleo Swift para Rust. Ele reúne as regras de texto, o dicionário, o histórico, as reuniões e a recuperação de gravações interrompidas. Os dois núcleos executam os casos de teste compartilhados em `contrato/casos/`.
- **`motores/`** executa o Parakeet via sherpa-onnx e separa os falantes. A limpeza e o resumo usam llama.cpp na CPU, com servidor em `127.0.0.1`, TLS e chave gerada a cada execução.
- **GitHub Actions** compila e testa o aplicativo e gera o instalador NSIS. O fluxo confere o SHA-256 das bibliotecas de terceiros baixadas e testa a regressão da taxa de erro de palavras (WER) do Parakeet.
- **Atualizador** verifica a assinatura minisign do instalador indicado no `latest.json` do canal estável. A chave privada fica fora do repositório.

### Compilar e testar no Windows

Em um Windows x64 com Rust (MSVC), ferramentas de build do Visual Studio, WebView2 e Node.js com npm, rode no Git Bash, a partir da raiz do repositório:

```bash
powershell.exe -NoProfile -File scripts/windows/preparar-vendor.ps1
sherpa_lib="$(find target/ditase-extract-sherpa -name sherpa-onnx-c-api.lib -print -quit)"
sherpa_dir="$(dirname "$sherpa_lib")"
export SHERPA_ONNX_LIB_DIR="$(cygpath -w "$sherpa_dir")"
export PATH="$(cd "$sherpa_dir" && pwd):$PATH"

(cd ui && npm ci && npm run build)
cargo test --locked --workspace --all-targets --release --features ditase/custom-protocol

npm ci --prefix scripts/windows/tauri-cli --ignore-scripts --no-audit --no-fund
cd windows
node ../scripts/windows/tauri-cli/node_modules/@tauri-apps/cli/tauri.js build --bundles nsis --ci \
  --config src-tauri/tauri.distribuicao.conf.json \
  --config '{"bundle":{"createUpdaterArtifacts":false,"windows":{"nsis":{"installMode":"currentUser"}}}}' -- --locked
```

O instalador local sai em `target/release/bundle/nsis/`, na raiz do projeto. Veja também [os detalhes da versão Windows](windows/README.md) e [o fluxo de CI](.github/workflows/ci.yml).

### Pré-requisitos para compilar no Mac

- Xcode 16+ e ferramentas de linha de comando do Xcode
- [XcodeGen](https://github.com/yonaskolb/XcodeGen) (`brew install xcodegen`)
- Node.js 20+ e npm
- `llama.cpp` (`brew install llama.cpp`) para limpeza com IA e resumo local, com modelos `.gguf` colocados em `~/Library/Application Support/Ditado/Models` (ex.: `Qwen3.5-2B-Q4_K_M.gguf`)

### Compilar e rodar no Mac

```bash
# 1. Instalar dependências e gerar o build da interface web
cd ui
npm ci
npm run build
cd ..

# 2. Compilar o aplicativo macOS (Debug ou Release)
scripts/compilar.sh Release

# 3. (Opcional) Assinar com identidade local estável e instalar em /Applications
scripts/instalar.sh
```

Para preservar um identificador de bundle personalizado na sua máquina sem alterar o `project.yml` versionado, crie um arquivo `local.local.xcconfig` na raiz (ignorado pelo Git) com `PRODUCT_BUNDLE_IDENTIFIER = seu.identificador`: o `scripts/instalar.sh` o aplica automaticamente.

### Rodar os testes no Mac

```bash
swift test --package-path Nucleo
```

### Linha de comando (sem abrir a interface)

O binário compilado aceita modos de linha de comando para testes automatizados e avaliação:

```bash
# Transcrever um arquivo WAV curto de ditado
Dita.se.app/Contents/MacOS/Dita.se --transcrever audio.wav --sem-ia

# Transcrever arquivo de áudio ou vídeo com separação de falantes (devolve JSON)
Dita.se.app/Contents/MacOS/Dita.se --transcrever-arquivo gravacao.mp4 --falantes --sem-ia

# Simular reunião a partir de arquivos de áudio do sistema e do microfone
Dita.se.app/Contents/MacOS/Dita.se --reuniao-simulada sistema.mp4 microfone.wav --sem-resumo
```

Mais detalhes técnicos sobre o contrato da ponte WebKit, o modelo de persistência de reuniões e as especificações de movimento estão na pasta [`docs/`](docs/).

</details>

## Licença

Distribuído sob a licença [MIT](LICENSE).
