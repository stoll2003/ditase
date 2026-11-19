# Dita.se

Ditado por voz, transcrição de reuniões e transcrição de arquivos de áudio e vídeo com separação de falantes para macOS. Tudo roda localmente no seu Mac, sem conta, sem assinatura e sem enviar áudio ou texto para servidores externos.

![Início](docs/screens-publicas/inicio-claro.png)

## O que é

O **Dita.se** transforma fala em texto direto no campo onde você está digitando e também transcreve chamadas e gravações longas:

- **Ditado rápido em qualquer aplicativo:** segure um atalho (por padrão a tecla `Opção direita ⌥`), fale e solte. O texto entra no cursor já pontuado e sem hesitações vazias, preservando as palavras que você disse. Também há modo mãos livres pelo botão flutuante acima do Dock.
- **Reuniões com falantes e resumo local:** captura o áudio do Mac junto com o seu microfone durante chamadas (Meet, Zoom, Teams ou qualquer app), mostra a transcrição quase ao vivo, identifica quem falou ao encerrar e gera um resumo curto no modelo local.
- **Arquivos de áudio e vídeo:** arraste gravações (`mp3`, `m4a`, `wav`, `aac`, `flac`, `ogg`, `mp4`, `mov`, `m4v`, `webm`) para separar as vozes por trecho, renomear participantes e exportar em texto (`.txt`), Markdown (`.md`) ou legendas (`.srt` e `.vtt`).
- **Dicionário pessoal e estatísticas:** cadastre nomes próprios, siglas e termos técnicos para que saiam sempre com a grafia certa, inclusive por semelhança fonética.
- **Fidelidade ao que foi dito:** a limpeza opcional por modelo local só pontua, organiza parágrafos e remove muletas ou autocorreções; uma camada determinística impede que o modelo invente frases ou troque suas palavras por sinônimos.

![Reunião](docs/screens-publicas/reuniao-claro.png)

![Arquivos](docs/screens-publicas/arquivos-claro.png)

![Ajustes](docs/screens-publicas/ajustes-claro.png)

## Requisitos

- Mac com chip Apple (M1 ou superior)
- macOS 15 ou superior

### Windows

Ainda não há versão para Windows. O motor de reconhecimento e diarização atual usa Core ML e FluidAudio, que rodam apenas em hardware Apple.

## Instalar no Mac

### Opção 1: instalar por uma linha no Terminal (recomendado)

Abra o aplicativo **Terminal** e cole:

```bash
curl -fsSL https://raw.githubusercontent.com/stoll2003/ditase/main/scripts/install.sh | bash
```

O script baixa o `.dmg` da última versão publicada no GitHub Releases, confere a integridade pelo hash SHA-256, copia o `Dita.se.app` para `/Applications` (ou `~/Applications` se a sua conta não for administradora) e remove o atributo de quarentena do download para que o app abra direto pelo ícone no Spotlight ou no Launchpad.

### Opção 2: baixar o DMG manualmente

1. Baixe o arquivo `Dita.se-mac-arm64.dmg` na página de **Releases** deste repositório.
2. Abra o `.dmg` e arraste o **Dita.se** para a pasta **Aplicativos**.
3. **Na primeira abertura:** como o aplicativo é distribuído com assinatura local gratuita (sem certificado pago da Apple), o macOS pode exibir um aviso dizendo que a Apple não pôde verificar o app. Para autorizar:
   - Tente abrir o **Dita.se** uma vez e feche o aviso.
   - Abra **Ajustes do Sistema** > **Privacidade e Segurança**.
   - Role até a seção **Segurança** e clique em **Abrir Mesmo Assim** ao lado da mensagem sobre o Dita.se.
   - Confirme com a senha ou Touch ID do Mac. Nas próximas vezes ele abrirá normalmente.

## Permissões necessárias

Na primeira vez que você usar cada função, o macOS pedirá as permissões correspondentes:

- **Microfone:** para ouvir a sua voz nos ditados e nas reuniões.
- **Acessibilidade:** para detectar o atalho global de teclado e colar o texto no campo em foco.
- **Gravação de Áudio do Sistema / Tela:** solicitada apenas quando você inicia uma reunião na aba **Reunião**, para ouvir os outros participantes da chamada. Nenhuma imagem da sua tela é gravada.

## Privacidade: tudo fica no seu Mac

- O reconhecimento de fala (**Parakeet Ultra** via FluidAudio), a separação de vozes e o modelo de limpeza e resumo (**Qwen** via `llama-server` em `127.0.0.1`) rodam inteiramente no seu computador.
- As únicas conexões de rede são para baixar os modelos na preparação do ambiente. Nenhum áudio, transcrição, termo do dicionário ou estatística sai do seu Mac.
- Os dados ficam guardados em `~/Library/Application Support/Ditado`.

## Desinstalar

1. Feche o Dita.se e mova `/Applications/Dita.se.app` (ou `~/Applications/Dita.se.app`) para a Lixeira.
2. Se quiser apagar também o histórico, o dicionário, as reuniões e o modelo de limpeza, remova a pasta `~/Library/Application Support/Ditado`.
3. O modelo de reconhecimento de fala e o de separação de vozes ficam num cache da biblioteca FluidAudio, em `~/Library/Application Support/FluidAudio/Models`. Essa pasta não tem nenhum dado seu e pode ser usada por outros apps que usam a mesma biblioteca. Se nenhum outro app precisar dela, pode removê-la também.

<details>
<summary><strong>Seção avançada: compilação a partir do código-fonte, arquitetura e testes</strong></summary>

### Arquitetura do projeto

- **`Nucleo/` (pacote Swift independente de interface):** segmentador de áudio por pausas, sessão de ditado, regras determinísticas de fala (`RegrasFala`), dicionário fonético (`AplicadorDicionario`), travas contra alucinação ou reescrita do modelo (`Guarda` e `Fidelidade`), persistência transacional de ditados, arquivos e reuniões em SQLite (`Banco`, `ArmazemLancamento`, `Costura`) e cálculo de estatísticas.
- **`Ditado/` (aplicativo macOS em AppKit + SwiftUI):** captura de atalho por event tap, gravação de microfone e de áudio do sistema (ScreenCaptureKit), decodificação de mídia (AVFoundation), integração com FluidAudio (Parakeet Ultra e Sortformer), gerenciamento sob demanda do processo `llama-server`, pílula flutuante e ponte WebKit (`ditado://`).
- **`ui/` (React 19 + TypeScript + Vite + Tailwind CSS 4 + Tuff UI):** interface da janela principal com as abas Início, Reunião, Arquivos, Insights, Dicionário e Ajustes.
- **`project.yml`:** especificação do XcodeGen que gera `Ditado.xcodeproj`.

### Pré-requisitos para compilar

- Xcode 16+ e ferramentas de linha de comando do Xcode
- [XcodeGen](https://github.com/yonaskolb/XcodeGen) (`brew install xcodegen`)
- Node.js 20+ e npm
- `llama.cpp` (`brew install llama.cpp`) para limpeza com IA e resumo local, com modelos `.gguf` colocados em `~/Library/Application Support/Ditado/Models` (ex.: `Qwen3.5-2B-Q4_K_M.gguf`)

### Compilar e rodar localmente

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

### Rodar os testes

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
