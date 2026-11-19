# Especificação Visual e de Experiência: Lançamento do Dita.se (Arquivos, Reunião e Pílula)

- **Projeto:** Dita.se (`Ditado/` e `ui/`)
- **Alvo:** Interface principal em React 19 + Tailwind 4 + Tuff UI (`ui/src/`) e pílula nativa em AppKit/SwiftUI (`Ditado/Pilula.swift`)
- **Fontes de contrato:** `docs/motion.md`, `docs/contrato-ponte-lancamento.md` e código de `ui/src/` e `Ditado/Pilula.swift`
- **Esboços visuais:** diagramas ASCII na seção 7

---

## 1. Navegação Lateral, Rodapé de Situação e Arrastar Global

### 1.1 Ordem das Seções na Barra Lateral (`ui/src/App.tsx`)

A barra lateral (`w-[13rem]`) passa de 4 para 6 itens, agrupando no topo as três formas de capturar fala (no cursor, em chamada e por arquivo) e abaixo a consulta e configuração:

| Ordem | `id` | Rótulo | Ícone (`@phosphor-icons/react`) | Indicador à direita do rótulo (quando ativo em segundo plano) |
|---|---|---|---|---|
| 1 | `inicio` | `Início` | `House` | Nenhum |
| 2 | `reuniao` | `Reunião` | `UsersThree` | Enquanto grava reunião: ponto `size-1.5 rounded-full bg-negative` + tempo `tabular-nums text-[0.6875rem] text-muted-foreground` (`14:28`). Quando pausada: ponto `bg-warning` + `Pausada`. |
| 3 | `arquivos` | `Arquivos` | `FileAudio` | Enquanto processa fila: texto `tabular-nums text-[0.6875rem] text-brand` (`48%` ou `1/3`). |
| 4 | `insights` | `Insights` | `ChartBar` | Nenhum |
| 5 | `dicionario` | `Dicionário` | `BookBookmark` | Nenhum |
| 6 | `ajustes` | `Ajustes` | `GearSix` | Nenhum |

- Todos os itens seguem o mesmo estilo atual de `App.tsx`: `h-8 w-full items-center gap-2.5 rounded-lg px-2 text-[0.8125rem] font-medium`, com o fundo deslizante `layoutId="nav-ativo"` (`bg-foreground/[0.08]`) e peso `fill` no ícone ativo.

### 1.2 Rodapé de Situação da Lateral (`Situacao` em `App.tsx`)

Amplia os estados atuais para refletir reunião e fila de arquivos sem esconder o estado do reconhecedor:

| Prioridade | Condição | Cor do ponto | Linha 1 (`font-medium text-foreground`) | Linha 2 (`text-muted-foreground`) |
|---|---|---|---|---|
| 1 | Reunião gravando | `bg-negative` (pulsa) | `Em reunião · 14:28` | `Áudio do Mac e microfone` (ou `Só áudio do Mac`) |
| 2 | Reunião pausada | `bg-warning` (fixo) | `Reunião pausada · 14:28` | `Clique em Reunião para retomar` |
| 3 | Reunião finalizando (diarização final / resumo) | `bg-brand` (fixo) | `Fechando a reunião` | `Separando vozes e resumindo` |
| 4 | Ditado gravando / processando | `bg-negative` / `bg-brand` | `Ouvindo` / `Ouvindo, mãos livres` / `Transcrevendo` | `Segure <atalho>` |
| 5 | Arquivo sendo processado | `bg-brand` (fixo) | `Transcrevendo arquivo · 48%` | `1 de 3 na fila` (ou nome curto do arquivo) |
| 6 | Permissão essencial de ditado ausente | `bg-warning` | `Falta permissão` | `Segure <atalho>` |
| 7 | Repouso normal | `bg-positive` | `Pronto para ditar` | `Segure <atalho>` |

### 1.3 Arrastar Arquivo sobre Qualquer Página (Global Drop Target)

Conforme `hig-drag-and-drop`, o usuário pode arrastar um ou vários arquivos do Finder para qualquer página aberta da janela (`Início`, `Reunião`, `Arquivos`, `Insights`, `Dicionário` ou `Ajustes`) e também conta sempre com o botão `"Escolher arquivo"` como alternativa sem arrasto:

1. **Arrastar sobre qualquer página exceto `Arquivos`:**
   - Enquanto o cursor com arquivo(s) estiver sobre a janela (`onDragOver`), o container `<main>` exibe uma camada sobreposta com margem de `8 px` (`absolute inset-2 z-40 flex flex-col items-center justify-center gap-2 rounded-2xl border-2 border-dashed border-brand bg-surface-inset/92 backdrop-blur-xs`):
     - `IconTile` com ícone `FileAudio` (`size-10 text-brand bg-brand/12`).
     - Título (`text-[0.9375rem] font-medium text-foreground`): `"Solte para transcrever com separação de vozes"`
     - Subtítulo (`text-[0.8125rem] text-muted-foreground`): `"Áudio ou vídeo (mp3, m4a, wav, aac, flac, ogg, mp4, mov, m4v, webm)"`
   - Ao soltar (`onDrop`), a interface troca para a página `Arquivos` (`irPara("arquivos")`), adiciona os arquivos na fila na ordem recebida e mantém o primeiro arquivo solto em foco/expandido.
2. **Spring loading na barra lateral (`hig-drag-and-drop`):**
   - Manter um arquivo arrastado sobre o botão `Arquivos` da barra lateral por `350 ms` ativa a página `Arquivos` sem precisar soltar primeiro.
3. **Arrastar já dentro da página `Arquivos`:**
   - Não cobre a lista com overlay de tela cheia; apenas acende a zona de soltar do topo da página (`border-brand bg-brand/[0.06]`).

---

## 2. Página "Arquivos" (Transcrição de Áudio e Vídeo com Falantes)

Segue a estrutura padrão de página (`mx-auto max-w-[52rem] px-8 pt-9 pb-12`).

### 2.1 Cabeçalho e Entrada de Arquivos

- **`Cabecalho`:**
  - `titulo`: `"Arquivos"`
  - `apoio`: `"Transcreva gravações de áudio ou vídeo e separe quem falou em cada trecho. Tudo roda neste Mac."`
  - `lado`: `<Button variant="primary" size="md" icon={FolderOpen} onClick={escolherArquivos}>Escolher arquivo</Button>` (abre `NSOpenPanel` nativo com `allowsMultipleSelection = true`; atalho `Cmd-O` quando a janela está na página `Arquivos`).
- **Formatos aceitos pelo filtro do `NSOpenPanel` e pelo drop:**
  - Áudio: `mp3`, `m4a`, `wav`, `aac`, `flac`, `ogg` (se suportado pelo AVFoundation do sistema).
  - Vídeo (o app lê apenas a trilha sonora e converte para 16 kHz mono): `mp4`, `mov`, `m4v`, `webm` (se suportado pelo sistema).
  - Se o usuário soltar qualquer outro arquivo (ou um `.ogg`/`.webm` sem codec nativo no macOS), o arquivo ainda entra no topo da lista no estado **Formato não suportado**, para que a rejeição nunca seja silenciosa (`hig-drag-and-drop`: *"Avoid: drops that silently fail"*).
- **Zona de Soltar (`DropZone`):**
  - **Quando não há nenhum arquivo na lista (estado inicial vazio):**
    - Bloco clicável e focável por teclado (`role="button"`, `tabIndex={0}`, `Enter` ou `Espaço` aciona `escolherArquivos`) com `min-h-[13.5rem] rounded-2xl border border-dashed border-foreground/15 bg-foreground/[0.02] hover:border-foreground/25 hover:bg-foreground/[0.04] focus-visible:ring-2 focus-visible:ring-ring p-8 flex flex-col items-center justify-center text-center transition-colors`:
    - Ícone `FileAudio` dentro de `IconTile` (`size-10 rounded-2xl`).
    - Título (`mt-3 text-[0.9375rem] font-medium`): `"Arraste arquivos de áudio ou vídeo para cá"`
    - Descrição (`mt-1 max-w-md text-[0.8125rem] leading-5 text-muted-foreground`): `"Aceita mp3, m4a, wav, aac, flac, ogg, mp4, mov, m4v e webm. Você pode soltar vários de uma vez ou usar Escolher arquivo."`
  - **Quando já existem arquivos na lista (modo compacto no topo):**
    - Faixa de `h-13 rounded-xl border border-dashed border-foreground/15 bg-foreground/[0.02] hover:border-foreground/25 px-4 mb-6 flex items-center justify-between gap-4`:
    - À esquerda: ícone `FileAudio` (`size-4 text-muted-foreground`) + texto `"Arraste mais arquivos para cá para pôr na fila"` (`text-[0.8125rem] font-medium`).
    - À direita: `<span className="text-[0.75rem] text-muted-foreground">mp3 · m4a · wav · flac · mp4 · mov</span>`.

### 2.2 Fila de Vários Arquivos e Progresso por Arquivo

Quando vários arquivos são adicionados, o motor processa um por vez em ordem de chegada. Todos aparecem imediatamente na seção `"Arquivos"` (com `<SearchField placeholder="Buscar nos arquivos" />` no topo da seção quando houver 2 ou mais arquivos concluídos).

- **Cabeçalho da fila quando há 2 ou mais arquivos pendentes/processando:**
  - Acima da lista, uma barra discreta (`flex items-center justify-between text-[0.75rem] text-muted-foreground mb-2 px-1`):
    - Texto à esquerda: `"Processando 1 de 3 arquivos"`
    - Ação à direita: botão `<Button variant="quiet" size="sm" onClick={cancelarFila}>Cancelar fila</Button>`
- **Cartão de cada arquivo (`Shell` com `ShellHeader` e `Panel`):**
  - **Linha superior (`ShellHeader`):**
    - Ícone: `IconTile` com `FileAudio` (para extensões de áudio) ou `FileVideo` (para `mp4`, `mov`, `m4v`, `webm`).
    - `title`: nome do arquivo com truncamento no meio se passar de 48 caracteres (`audiencia-instrucao-2026-11-18.mp4`).
    - `subtitle` (em `tabular-nums`):
      - Em andamento / na fila: duração do arquivo (quando já lida) + descrição curta da etapa atual (ver tabela de estados na seção 2.3).
      - Quando pronto: `"18 de nov. · 14:20 · 42 min 18 s · 3 vozes · 5.420 palavras · pronto em 19 s"`.
    - `aside` (controles à direita do cabeçalho):
      - Enquanto está na fila ou processando: `StatusPill` do estado + `IconButton` (`X`, `label="Cancelar este arquivo"`, `tooltip="Cancelar"`).
      - Quando pronto:
        1. `<Button variant="secondary" size="sm" icon={copiado ? Check : Copy} onClick={copiarTudo}>{copiado ? "Copiado" : "Copiar"}</Button>`
        2. Menu `<Menu trigger={<Button variant="secondary" size="sm" icon={DownloadSimple}>Exportar</Button>} ... />`
        3. `<IconButton icon={aberto ? CaretUp : CaretDown} label={aberto ? "Recolher transcrição" : "Mostrar transcrição"} />`
        4. Menu `Mais ações` (`DotsThree`): `"Transcrever de novo"` (`ArrowClockwise`), `"Ver o que foi ouvido"` (`TextAa`), `"Apagar da lista"` (`Trash`, destrutivo).
  - **Barra de progresso individual (visível dentro do cartão apenas nos estados `decodificando`, `transcrevendo` e `identificando falantes`):**
    - Componente `ProgressBar` do Tuff UI (`size="sm"`, `tone="brand"`, `value={progressoPercentual}`, `label="Progresso do arquivo"`), posicionado em `px-3 pb-2.5`.

### 2.3 Todos os Estados de um Arquivo

| Estado | `StatusPill` (`tone`, ícone, texto) | Subtítulo / Mensagem no cartão | Barra de progresso | Ações disponíveis |
|---|---|---|---|---|
| **Na fila** (`naFila`) | `neutral`, `Clock`, `"Na fila"` | `"Aguardando o arquivo anterior terminar"` | Oculta | `IconButton` (`X`, `"Tirar da fila"`) |
| **Decodificando** (`decodificando`) | `brand`, sem ícone fixo, `"Lendo o áudio · 8%"` | `"Convertendo o arquivo para 16 kHz mono"` | Visível (`0%` a `10%`) | `IconButton` (`X`, `"Cancelar"`) |
| **Transcrevendo** (`transcrevendo`) | `brand`, sem ícone, `"Transcrevendo · 54%"` | `"Reconhecendo a fala no Parakeet Ultra"` | Visível (`10%` a `80%`) | `IconButton` (`X`, `"Cancelar"`) |
| **Identificando falantes** (`diarizando`) | `brand`, sem ícone, `"Separando vozes · 88%"` | `"Identificando quem falou em cada trecho"` | Visível (`80%` a `100%`) | `IconButton` (`X`, `"Cancelar"`) |
| **Pronto** (`pronto`) | Sem pílula (mostra metadados completos no subtítulo) | `"42 min 18 s · 3 vozes · 5.420 palavras · pronto em 19 s"` | Oculta | `Copiar`, `Exportar`, `Expandir/Recolher`, `Mais ações` (`Transcrever de novo`, `Ver o que foi ouvido`, `Apagar`) |
| **Formato não suportado** (`formatoNaoSuportado`) | `warning`, `Warning`, `"Formato não suportado"` | `"O macOS não conseguiu abrir este formato. Use mp3, m4a, wav, aac, flac, mp4 ou mov."` | Oculta | `IconButton` (`Trash`, `"Remover da lista"`) |
| **Arquivo sem fala** (`semFala`) | `neutral`, `Info`, `"Sem fala detectada"` | `"O áudio foi lido até o fim, mas o reconhecedor não encontrou voz audível."` | Oculta | `Button` `"Transcrever de novo"` + `IconButton` (`Trash`, `"Apagar"`) |
| **Falha** (`falhou`) | `negative`, `WarningCircle`, `"Falhou"` | `"Não foi possível concluir a transcrição deste arquivo."` (mais o motivo curto se o arquivo sumiu ou corrompeu) | Oculta | `Button` `"Tentar de novo"` (`ArrowClockwise`) + `IconButton` (`Trash`, `"Apagar"`) |

### 2.4 Resultado por Falante, Renomeação, Cores e Carimbo de Tempo

Quando o arquivo está no estado **Pronto** e aberto (`Panel` interno do `Shell`, com `p-4`):

1. **Faixa de Falantes (topo do resultado):**
   - Linha `flex flex-wrap items-center gap-1.5 pb-3 mb-3 border-b border-foreground/[0.06]`:
   - Rótulo à esquerda: `"Vozes:"` (`text-[0.75rem] font-medium text-muted-foreground mr-1`).
   - Um chip por falante detectado (`Pessoa 1`, `Pessoa 2`, `Pessoa 3`…):
     - Cada chip (`h-7 rounded-full px-2.5 text-[0.75rem] font-medium inline-flex items-center gap-1.5 transition-colors`) traz o ponto colorido de `8 px` (`size-2 rounded-full`), o nome do falante, a contagem de falas ou tempo (`"14 falas"`, `text-muted-foreground font-normal`) e o ícone `PencilSimple` (`size-3 opacity-60`).
   - **Renomear falante em linha:**
     - Clicar no chip de qualquer falante (na faixa do topo ou no cabeçalho de um bloco de fala) transforma o chip num campo de texto curto em linha (`input` com `h-7 w-36 rounded-full bg-surface px-2.5 text-[0.75rem] font-medium text-foreground outline-none ring-2 ring-ring`, valor inicial selecionado).
     - Pressionar `Enter` ou clicar fora (`onBlur`) salva o novo nome e atualiza todos os blocos daquele falante no arquivo (mostrando `toast.success('Renomeado para "Dr. Silva"')`). Pressionar `Esc` cancela. Se o usuário salvar dois falantes com o mesmo nome (ex.: juntar `Pessoa 2` e `Pessoa 3` que eram a mesma pessoa), o sistema unifica os dois sob aquele nome e cor.

2. **Paleta Discreta por Falante (Claro e Escuro):**
   - Não usa bordas laterais grossas (`craft-floor`). A cor identifica o falante pelo ponto de `8 px` (`size-2 rounded-full`) e pelo fundo suave da pílula do nome, mantendo contraste de texto `>= 4,5:1` nos dois temas. Além da cor, o nome textual está sempre presente (`hig-accessibility`):

| Slot de Falante | Uso padrão | Tema Claro (Ponto / Texto / Fundo da pílula) | Tema Escuro (Ponto / Texto / Fundo da pílula) |
|---|---|---|---|
| **Voz 1** (ou `Você` na reunião) | Primeiro falante ou microfone local | Ponto `#0071e3` · Texto `#004b99` · Fundo `rgba(0, 113, 227, 0.11)` | Ponto `#0a84ff` · Texto `#7cc0ff` · Fundo `rgba(10, 132, 255, 0.16)` |
| **Voz 2** | Segundo falante (`Pessoa 1` ou `Pessoa 2`) | Ponto `#6e56cf` · Texto `#473299` · Fundo `rgba(110, 86, 207, 0.11)` | Ponto `#9b8afb` · Texto `#c7beff` · Fundo `rgba(155, 138, 251, 0.16)` |
| **Voz 3** | Terceiro falante | Ponto `#0f8a7e` · Texto `#085e56` · Fundo `rgba(15, 138, 126, 0.11)` | Ponto `#2dd4bf` · Texto `#7ef3e3` · Fundo `rgba(45, 212, 191, 0.16)` |
| **Voz 4** | Quarto falante | Ponto `#c96a00` · Texto `#7d4000` · Fundo `rgba(201, 106, 0, 0.11)` | Ponto `#f59e0b` · Texto `#ffd080` · Fundo `rgba(245, 158, 11, 0.16)` |
| **Voz 5** | Quinto falante | Ponto `#c83e73` · Texto `#872048` · Fundo `rgba(200, 62, 115, 0.11)` | Ponto `#f472b6` · Texto `#ffb3d9` · Fundo `rgba(244, 114, 182, 0.16)` |
| **Voz 6+ / Não identificada** | Sexto em diante ou baixa confiança | Ponto `var(--muted-foreground)` · Texto `var(--foreground)` · Fundo `bg-foreground/[0.07]` | Ponto `var(--muted-foreground)` · Texto `var(--foreground)` · Fundo `bg-foreground/[0.08]` |

3. **Estrutura de cada Bloco de Fala:**
   - Segmentos consecutivos do mesmo falante são unidos num mesmo bloco enquanto a pausa entre eles for menor que `8 s`.
   - Cada bloco é um item de lista (`rounded-xl px-3 py-2.5 transition-colors hover:bg-foreground/[0.025] group`):
     - **Linha de topo do bloco (`flex items-center justify-between gap-2 mb-1`):**
       - À esquerda: botão-pílula do falante (`h-5 rounded-full px-2 text-[0.75rem] font-semibold inline-flex items-center gap-1.5`, clicável para renomear, `title="Clique para mudar o nome"`) + carimbo de tempo em `font-mono text-[0.6875rem] text-muted-foreground tabular-nums` (`00:14` ou `01:02:14` quando o áudio tem 1 hora ou mais).
       - À direita (aparece em `opacity-0 group-hover:opacity-100 focus-within:opacity-100`): `<IconButton icon={Copy} size="sm" variant="quiet" label="Copiar este trecho" tooltip="Copiar trecho" />`.
     - **Corpo do bloco:** `<p className="selecionavel text-[0.875rem] leading-6 whitespace-pre-wrap text-foreground">...</p>`.
   - Se o arquivo tiver apenas **1 único falante** detectado do início ao fim: a faixa de vozes mostra `"1 voz detectada (Pessoa 1)"` (ainda renomeável caso o usuário queira exportar com o nome da pessoa), e os blocos mantêm o carimbo de tempo por parágrafo.

### 2.5 Opções de Copiar e Exportar (Decisão Fechada: Texto, Markdown, `.srt` e `.vtt`)

Como a página aceita arquivos de áudio e de **vídeo** (`.mp4`, `.mov`, `.m4v`, `.webm`) e o reconhecedor produz o início e o fim de cada segmento, a exportação cobre tanto documentos de texto quanto arquivos de legenda padrão:

- **Botão `Copiar`:** copia para a área de transferência todos os blocos no formato limpo:
  ```text
  [00:00] Pessoa 1
  Texto do primeiro bloco.

  [00:18] Pessoa 2
  Texto do segundo bloco.
  ```
  Dispara `toast.success("Transcrição copiada")`.
- **Menu `Exportar` (`Menu` do Tuff UI com 2 grupos):**
  - **Grupo 1 (Área de transferência):**
    - `Copiar com falantes e tempos` (`Copy`)
    - `Copiar só o texto corrido` (`TextAlignLeft`, sem cabeçalhos de falante nem carimbos de tempo)
  - **Grupo 2 (Salvar arquivo via `NSSavePanel`):**
    - `Salvar texto (.txt)` (mesmo formato com `[00:00] Nome`)
    - `Salvar documento Markdown (.md)` (cabeçalho `# Nome do arquivo`, metadados de data e duração, resumo se for reunião, e blocos `**[00:00] Nome:** Texto`)
    - `Salvar legendas SubRip (.srt)` (segmentos curtos com tempos `00:00:00,000 --> 00:00:04,200` e prefixo `[Nome] ` quando houver mais de 1 voz)
    - `Salvar legendas WebVTT (.vtt)` (cabeçalho `WEBVTT` e tags `<v Nome>Texto`)

---

## 3. Aba "Reunião" (Transcrição Quase ao Vivo, Falantes e Resumo Local)

Segue o mesmo container de página (`mx-auto max-w-[52rem] px-8 pt-9 pb-12`).

### 3.1 Estado 1: Antes de Começar (Prontidão das Fontes e Permissões)

- **`Cabecalho`:**
  - `titulo`: `"Reunião"`
  - `apoio`: `"Transcreve chamadas deste Mac junto com a sua voz e gera um resumo local ao terminar, sem entrar como participante na chamada."`
  - `lado`: `<Button variant="primary" size="md" icon={Microphone} onClick={iniciarReuniao} disabled={estado?.fase === "gravando"}>Começar reunião</Button>` (se já houver um ditado rápido em curso, fica desabilitado até o ditado soltar).
- **Aviso de Permissão Pendente (aparece acima do painel de fontes somente quando falta permissão de Áudio do Sistema/Tela ou Microfone):**
  - Usa o mesmo padrão visual de `Pendencias` em `Inicio.tsx` (`Inset className="flex items-center gap-3 py-3 pr-3 pl-4 mb-4"`):
  - **Quando falta Gravação de Áudio do Sistema / Tela (`permissoes.audioSistema !== "concedida"`):**
    - Ícone: `Warning` (`weight="fill" className="size-4 shrink-0 text-warning"`).
    - Título (`text-[0.8125rem] font-medium`): `"Permita capturar o áudio do Mac"`
    - Texto (`text-[0.8125rem] text-muted-foreground`): `"É o que deixa o Dita.se ouvir as outras pessoas da chamada (Meet, Zoom, Teams). Nenhuma imagem da tela é gravada e o áudio não sai deste Mac."`
    - Botão: `<Button onClick={() => chamar("permissao.pedir", { qual: "audioSistema" })}>Permitir</Button>`
  - **Quando falta Microfone (`permissoes.microfone !== "concedida"`):**
    - Título: `"Permita o microfone"`
    - Texto: `"Para incluir a sua voz na transcrição da reunião como Você."`
    - Botão: `<Button onClick={() => chamar("permissao.pedir", { qual: "microfone" })}>Permitir</Button>`
- **Painel de Fontes da Próxima Reunião (`Shell` com `Panel` compacto de 1 linha, `p-4 mb-8`):**
  - Exibe lado a lado o que será gravado ao clicar em `"Começar reunião"`:
    - **Esquerda (`Select` ou controle de fonte):** `"Fontes de áudio"` com 2 opções:
      1. `"Áudio do Mac + meu microfone (recomendado)"`: separa o seu microfone como `"Você"` e as vozes do sistema como `"Pessoa 1, 2…"`.
      2. `"Só o áudio do Mac"`: útil para palestras, aulas ou quando você está com o microfone mutado.
    - **Direita (indicador de prontidão):** `StatusPill tone="positive" icon={CheckCircle}` `"Pronto para capturar"` (ou `tone="warning"` `"Falta permissão do áudio do Mac"`).

### 3.2 Estado 2: Durante a Reunião (Transcrição Quase ao Vivo)

Quando a reunião está ativa (`gravando` ou `pausada`), o topo da página `Reunião` vira o **Painel da Reunião ao Vivo**:

1. **Barra de Controle ao Vivo (`Shell` fixo no topo da página):**
   - **`ShellHeader`:**
     - À esquerda:
       - Ponto de status (`size-2.5 rounded-full bg-negative` pulsando a cada `1,4 s` enquanto grava; `bg-warning` fixo quando pausada).
       - Campo de título editável em linha (`text-[0.9375rem] font-semibold`, valor padrão `"Reunião de hoje, 10:52"`, placeholder `"Dar nome à reunião"`).
       - Contador de **tempo decorrido** ao lado do título em `font-mono text-[0.9375rem] font-medium text-muted-foreground tabular-nums` (`14:28`).
     - No centro / subtítulo (`flex flex-wrap items-center gap-2 mt-0.5`):
       - Fontes ativas: `"Áudio do Mac + microfone (Você)"`.
       - **Indicador de atraso do fluxo (`StatusPill size="sm"`):**
         - Atraso normal (`<= 4 s`): `tone="neutral"`, texto `"Quase ao vivo · ~2 s de atraso"`
         - Atraso moderado/alto (`> 4 s`, quando o diarizador ou o Parakeet está alcançando a fala): `tone="warning"`, ícone `Clock`, texto `"Atraso de ~8 s · alcançando o áudio"`
         - Quando pausada: `tone="warning"`, texto `"Pausada"`
     - À direita (`aside`, botões de controle):
       - `<Button variant="secondary" size="md" icon={pausada ? Play : Pause} onClick={alternarPausa}>{pausada ? "Retomar" : "Pausar"}</Button>`
       - `<Button variant="danger" size="md" icon={Stop} onClick={encerrarReuniao}>Encerrar</Button>`

2. **Faixa de Participantes Detectados até Agora (abaixo do cabeçalho do `Shell`):**
   - Exibe os chips dos falantes já ouvidos (`Você`, `Pessoa 1`, `Pessoa 2`…). O usuário já pode clicar num chip para renomear (`Pessoa 1` -> `Marina`) enquanto a reunião ainda está acontecendo.

3. **Corpo da Transcrição Rolando (`Panel` com `max-h-[28rem] rolagem p-4`):**
   - Antes da primeira fala sair (primeiros segundos da chamada):
     - Linha discreta com `Spinner` / pontos: `"Ouvindo a chamada… Os primeiros trechos aparecem alguns segundos depois que alguém fala."`
   - Conforme os blocos chegam do motor em fluxo:
     - Cada bloco de falante entra com `LinhaAnimada` (`docs/motion.md`), exibindo a pílula colorida do falante (`Você`, `Pessoa 1`…), o carimbo de tempo relativo ao início (`02:15`) e o texto.
     - O trecho final ainda aberto (sendo falado nos últimos segundos) fica na última linha com texto em `text-muted-foreground` até a frase fechar.
   - **Rolagem automática inteligente:**
     - Enquanto o usuário estiver no fim da lista, novos blocos rolam a lista suavemente para baixo.
     - Se o usuário rolar para cima para revisar um trecho anterior ou renomear alguém, a rolagem automática para (para não tirar o texto da frente dele) e aparece um botão flutuante centralizado no rodapé do painel: `<Button variant="secondary" size="sm" icon={ArrowDown} onClick={irParaOFim}>Acompanhar ao vivo</Button>`.

### 3.3 Estado 3: Passada Final e Depois da Reunião (Resumo Local + Transcrição)

Ao clicar em **Encerrar** (na página ou na pílula):

1. **Fase de Fechamento (`finalizando`, dura poucos segundos):**
   - A gravação de áudio para imediatamente.
   - A transcrição completa já permanece visível e copiável.
   - No topo da reunião recém-encerrada, o cartão de **Resumo** exibe uma barra de progresso (`ProgressBar size="sm" tone="brand"`) e a legenda: `"Fazendo a revisão final das vozes e gerando o resumo no Qwen local…"` (com botão `"Cancelar resumo"` caso o usuário queira só a transcrição imediata).

2. **Visão da Reunião Concluída:**
   - **Cabeçalho da reunião:**
     - Título editável (`"Reunião de 18/11 às 10:52"`), metadados (`"Hoje, 10:52 · 38 min 12 s · 3 vozes (Você, Marina, Pessoa 2) · 4.810 palavras"`).
     - Ações à direita:
       - `<Button variant="primary" size="sm" icon={Copy} onClick={copiarReuniao}>Copiar tudo</Button>` (copia resumo + transcrição com falantes)
       - `<Menu trigger={<Button variant="secondary" size="sm" icon={DownloadSimple}>Exportar</Button>} ... />` (mesmas opções de Arquivos: `Copiar só o resumo`, `Copiar só a transcrição`, `Salvar Markdown (.md)`, `Salvar texto (.txt)`, `Salvar legendas (.srt)`, `Salvar WebVTT (.vtt)`)
       - Menu `Mais ações` (`DotsThree`): `"Gerar resumo de novo"`, `"Ver o que foi ouvido"`, `"Apagar reunião"` (`Trash`, destrutivo, com recolhimento da linha e toast `"Reunião apagada"` + botão `"Desfazer"`).
   - **Bloco do Resumo Local (`Inset` no topo da reunião):**
     - Separado visualmente da transcrição literal para cumprir a regra do `plano-lancamento.md` (*"O resumo é texto gerado, rotulado como resumo, separado da transcrição"*):
     - Cabeçalho do bloco (`flex items-center justify-between gap-2 mb-2`):
       - Título à esquerda: `"Resumo"` (`text-[0.8125rem] font-semibold text-foreground`) + selo `<Tag>Gerado localmente · Qwen</Tag>`
       - Ação à direita: `<IconButton icon={Copy} size="sm" variant="quiet" label="Copiar só o resumo" tooltip="Copiar resumo" />`
     - Corpo do resumo: lista enxuta de 3 a 5 pontos (`text-[0.875rem] leading-6 text-foreground selecionavel`) + nota de rodapé fixa em `text-[0.75rem] text-muted-foreground mt-2`: `"Resumo gerado por modelo local a partir da transcrição. Para trechos exatos, consulte as falas abaixo."`
     - **Se a limpeza por IA / `llama-server` estiver desligada ou sem modelo nos Ajustes:** o bloco de Resumo exibe apenas uma linha discreta: `"Resumo não gerado porque o modelo local (Qwen) está desligado nos Ajustes."` com botão `<Button variant="quiet" size="sm" onClick={gerarResumo}>Tentar gerar resumo</Button>`.
   - **Faixa de Falantes + Blocos da Transcrição Completa:**
     - Mesma faixa de chips renomeáveis da página `Arquivos` (`Você`, `Pessoa 1`, `Pessoa 2`…) + campo `<SearchField placeholder="Buscar nesta reunião" />` quando a reunião tiver mais de 10 blocos.

### 3.4 Estado de Baixa Confiança de Falantes (`baixaConfiancaFalantes`)

Quando o motor de diarização (tanto em `Arquivos` quanto em `Reunião`) detectar sobreposição frequente de vozes, volume muito baixo ou distância curta entre os timbres das pessoas do áudio do sistema:

1. **Nunca inventar trocas falsas de falante:** trechos onde o modelo não tem confiança mínima recebem o rótulo neutro `"Voz não identificada"` (ou permanecem agrupados como `"Participantes (áudio do Mac)"` quando nenhuma separação no canal do sistema atingir o limiar).
2. **Aviso explícito e acionável no topo da transcrição (`Inset` com fundo `bg-warning/[0.08]`, ícone `Info` `text-warning`, `py-2.5 px-3.5 mb-3`):**
   - **Texto durante a reunião:** `"Não foi possível separar bem as vozes em alguns trechos. Ao encerrar, o Dita.se fará uma revisão final da gravação."`
   - **Texto após concluir (reunião ou arquivo):** `"Não foi possível separar bem as vozes nesta gravação (vozes sobrepostas ou áudio distante). Clique no nome acima de qualquer trecho para indicar quem falou."`
3. **Reatribuição rápida de um trecho individual:**
   - Além de renomear um falante inteiro na faixa do topo, clicar no chip de falante **dentro de um bloco específico** abre um menu rápido (`Menu` do Tuff UI) com:
     - `"Mudar só este trecho para:"` listando os falantes existentes (`Você`, `Pessoa 1`, `Pessoa 2`…) + `"Nova pessoa…"`.
     - `"Renomear todas as falas de Pessoa 1…"` (aciona a edição do nome global daquele falante).

### 3.5 Lista de Reuniões Anteriores

Abaixo do painel de nova reunião (ou abaixo da reunião ativa), a seção `<Secao titulo="Reuniões anteriores">` lista todas as reuniões salvas no banco local, agrupadas por data (`Hoje`, `Ontem`, `14 de nov.`):
- A reunião mais recente recém-encerrada já nasce aberta.
- As anteriores aparecem como cartões `Inset` compactos (`py-3 pr-2 pl-4 flex items-center justify-between gap-3`), mostrando:
  - Título da reunião (`"Reunião de 18/11 às 10:52"`), primeira linha do resumo (ou início da transcrição se não houver resumo) em `line-clamp-2 text-[0.8125rem] text-muted-foreground`, e a linha de metadados em `tabular-nums text-[0.75rem] text-muted-foreground` (`"10:52 · 38 min 12 s · 3 vozes · 4.810 palavras"`).
  - Clicar no cartão expande a reunião completa (resumo, faixa de vozes e transcrição).

---

## 4. Botão Flutuante na Doca e Pílula (`Ditado/Pilula.swift`)

### 4.1 Decisão de Interação (HIG): Cápsula Dividida no Hover + Menu Contextual

Em `Ditado/Pilula.swift`, a pílula em repouso é uma marca de `40 × 8 pt` centrada acima do Dock.
- **Por que não usar apenas clique simples abrindo menu?** Hoje o clique simples no botão `"Ditar"` inicia imediatamente o ditado em mãos livres (comportamento central já documentado nos Ajustes). Abrir um menu a cada clique adicionaria um segundo clique obrigatório para ditar.
- **Por que não usar apenas clique direito?** A Apple HIG (`hig-pointing-devices` e `hig-the-menu-bar`) determina que menus de contexto (`Control-clique` / botão direito) são atalhos secundários e nunca o único caminho visível para uma função principal.
- **Solução adotada (1 clique direto + menu de botão direito):**
  1. **Em repouso (`sobre == false`):** permanece inalterada em **`40 × 8 pt`**.
  2. **Ao passar o ponteiro (`sobre == true`, `botaoAberto`):** a cápsula cresce de `40 × 8 pt` para **`176 × 32 pt`** (dentro do palco existente de `344 × 32 pt`; a `JanelaDaPilula.zona` aberta passa de `124 × 52 pt` para `196 × 52 pt`) e exibe **duas ações lado a lado** separadas por um divisor vertical de `1 × 14 pt` (`Color.white.opacity(0.16)`):
     - **Metade esquerda (`92 × 32 pt`, botão `"Ditar"`):** ícone `mic.fill` (`11 pt`, `.semibold`) + `"Ditar"` (`12 pt`, `.semibold`). `help`: `"Clique para ditar em mãos livres, ou segure ⌥ direita"`. `accessibilityLabel`: `"Começar a ditar em mãos livres"`.
     - **Metade direita (`83 × 32 pt`, botão `"Reunião"`):** ícone `waveform.and.mic` (`11 pt`, `.semibold`) + `"Reunião"` (`12 pt`, `.semibold`). `help`: `"Começar a transcrever uma reunião"`. `accessibilityLabel`: `"Começar a transcrever uma reunião"`.
  3. **Clique direito (`Control-clique`) em qualquer estado ocioso da pílula:** abre também um `NSMenu` nativo com `"Ditar em mãos livres"`, `"Começar reunião"`, `"Transcrever arquivo…"` e `"Abrir o Dita.se"`.

### 4.2 Como a Pílula Mostra que uma Reunião Está Sendo Transcrita

Novo caso em `ModeloDaPilula.Fase`: `.reuniao(pausada: Bool)`.
- **Dimensões da cápsula em `.reuniao`:** **`212 × 30 pt`** (`JanelaDaPilula.zona` = `230 × 50 pt`). A pílula permanece visível durante toda a reunião mesmo que `"Mostrar o botão flutuante quando parado"` esteja desligado nos Ajustes.
- **Conteúdo interno (`HStack(spacing: 8)` centrado na cápsula de `212 × 30 pt`):**
  1. **Botão de estado / abrir janela (`HStack(spacing: 6)` clicável):**
     - Ponto indicador de `6 × 6 pt`: vermelho vivo (`Color(red: 1, green: 0.27, blue: 0.23)`) enquanto grava; âmbar (`Color(red: 1, green: 0.62, blue: 0.04)`) quando pausada.
     - Rótulo `"Reunião"` (`11 pt`, `.semibold`, `.white`).
     - Quando gravando: componente `Barras` (ou versão compacta de 5 barras, `27 × 18 pt`) reagindo ao áudio capturado; quando pausada: texto `"Pausada"` em `11 pt` `Color.white.opacity(0.65)`.
     - Contador de tempo (`14:28`, ou `1:04:12` acima de 60 min) em `.system(size: 11, weight: .medium).monospacedDigit()`, `Color.white.opacity(0.75)`.
     - Clicar nessa área esquerda abre/traz para a frente a janela principal do Dita.se diretamente na aba `"Reunião"`. `help`: `"Abrir a transcrição da reunião no Dita.se"`.
  2. **Botão redondo Pausar / Retomar (`20 × 20 pt`, `BotaoRedondo` neutro):**
     - Símbolo `"pause.fill"` (quando gravando) ou `"play.fill"` (quando pausada).
     - `rotulo` acessível e `.help`: `"Pausar a transcrição da reunião"` / `"Retomar a transcrição da reunião"`.
  3. **Botão redondo Encerrar (`20 × 20 pt`, `BotaoRedondo` com `destaque: true` vermelho):**
     - Símbolo `"stop.fill"`.
     - `rotulo` acessível e `.help`: `"Encerrar a reunião"`.

### 4.3 Onde Aparece o Contorno Azul de Progresso na Pílula

O contorno azul (`Color(red: 0.04, green: 0.52, blue: 1.0)` / `#0A84FF`, traço de `2 pt` sobre a `Capsule()` preenchendo de `0%` a `100%` no sentido horário a partir de 12h, especificado em `docs/motion.md`) substitui/sobrepõe a borda branca discreta da pílula sempre que `modelo.progresso` (`Double?` de `0.0` a `1.0`) estiver presente:

| Situação | Fase e tamanho da pílula | Conteúdo interno da cápsula | Comportamento do contorno azul |
|---|---|---|---|
| **1. Processando arquivo(s) da aba Arquivos** (sem ditado nem reunião ativa) | `.processandoArquivo` (`136 × 30 pt`, zona `156 × 50 pt`, clicável para abrir a aba `Arquivos`) | Ícone `waveform` (`11 pt`) + texto `"Arquivo · 48%"` (`11 pt`, `.medium`, `.monospacedDigit()`) | Preenche no sentido horário de `0.0` a `1.0` conforme a fração processada do arquivo atual. Ao terminar, mostra `.aviso("Arquivo transcrito")` por `2,2 s`. |
| **2. Fechando reunião** (passada final de falantes + resumo no Qwen após clicar em Encerrar) | `.processando` ou `.fechandoReuniao` (`148 × 30 pt`) | Três pontos `Pontos` (compactos) + texto `"Resumindo · 75%"` (`11 pt`, `.medium`, `.monospacedDigit()`) | Preenche no sentido horário conforme a revisão final e o resumo avançam. Ao terminar, mostra `.aviso("Reunião pronta")` por `2,2 s`. |
| **3. Processando ditado de voz** | `.processando` (`64 × 30 pt`) | Três pontos em onda (`Pontos`) | Se o motor informar fração (`progresso != nil`), desenha o contorno azul ao redor da cápsula de `64 × 30 pt`; se `progresso == nil` (ditado curto rápido), mantém apenas os três pontos em onda sem contorno azul. |

---

## 5. Catálogo Fechado de Textos da Interface (Português do Brasil)

Todos os textos seguem a voz atual do Dita.se: frases curtas, diretas, sem jargão técnico desnecessário e sem pontuação artificial.

### 5.1 Página Arquivos
- Título da página: `Arquivos`
- Apoio do cabeçalho: `Transcreva gravações de áudio ou vídeo e separe quem falou em cada trecho. Tudo roda neste Mac.`
- Botão principal: `Escolher arquivo`
- Dropzone vazia (título): `Arraste arquivos de áudio ou vídeo para cá`
- Dropzone vazia (descrição): `Aceita mp3, m4a, wav, aac, flac, ogg, mp4, mov, m4v e webm. Você pode soltar vários de uma vez ou usar Escolher arquivo.`
- Dropzone compacta: `Arraste mais arquivos para cá para pôr na fila`
- Overlay de arrasto global (em outras abas): `Solte para transcrever com separação de vozes`
- Rótulos de progresso:
  - `Na fila` / `Aguardando o arquivo anterior terminar`
  - `Lendo o áudio · {n}%` / `Convertendo o arquivo para 16 kHz mono`
  - `Transcrevendo · {n}%` / `Reconhecendo a fala no Parakeet Ultra`
  - `Separando vozes · {n}%` / `Identificando quem falou em cada trecho`
- Estados de erro e exceção:
  - Formato não suportado: `Formato não suportado` / `O macOS não conseguiu abrir este formato. Use mp3, m4a, wav, aac, flac, mp4 ou mov.`
  - Sem fala: `Sem fala detectada` / `O áudio foi lido até o fim, mas o reconhecedor não encontrou voz audível.`
  - Falha: `Falhou` / `Não foi possível concluir a transcrição deste arquivo.`
- Toasts:
  - `Transcrição copiada`
  - `Trecho copiado`
  - `Renomeado para "{nome}"`
  - `Arquivo removido da lista` (com ação `Desfazer`)

### 5.2 Página Reunião
- Título da página: `Reunião`
- Apoio do cabeçalho: `Transcreve chamadas deste Mac junto com a sua voz e gera um resumo local ao terminar, sem entrar como participante na chamada.`
- Botão principal: `Começar reunião`
- Botões durante a reunião: `Pausar`, `Retomar`, `Encerrar`
- Indicadores de atraso:
  - `Quase ao vivo · ~{n} s de atraso`
  - `Atraso de ~{n} s · alcançando o áudio`
  - `Pausada`
- Aviso de permissão de áudio do sistema:
  - Título: `Permita capturar o áudio do Mac`
  - Texto: `É o que deixa o Dita.se ouvir as outras pessoas da chamada (Meet, Zoom, Teams). Nenhuma imagem da tela é gravada e o áudio não sai deste Mac.`
  - Botão: `Permitir`
- Aviso de baixa confiança de falantes:
  - Durante a reunião: `Não foi possível separar bem as vozes em alguns trechos. Ao encerrar, o Dita.se fará uma revisão final da gravação.`
  - Após concluir (reunião ou arquivo): `Não foi possível separar bem as vozes nesta gravação. Clique no nome acima de qualquer trecho para indicar quem falou.`
  - Rótulo de trecho incerto: `Voz não identificada`
- Bloco de resumo local:
  - Título: `Resumo`
  - Selo: `Gerado localmente · Qwen`
  - Nota de rodapé do resumo: `Resumo gerado por modelo local a partir da transcrição. Para trechos exatos, consulte as falas abaixo.`
  - Quando Qwen estiver desligado: `Resumo não gerado porque o modelo local (Qwen) está desligado nos Ajustes.`

### 5.3 Pílula e Botão Flutuante
- Botões no hover em repouso: `Ditar` | `Reunião`
- Durante reunião: `Reunião` + `{mm:ss}` (ou `Pausada`)
- Durante arquivo: `Arquivo · {n}%`
- Durante fechamento de reunião: `Resumindo · {n}%`
- Avisos curtos de fim (`2,2 s`): `Arquivo transcrito`, `Reunião pronta`

---

## 6. Acessibilidade (Teclado, VoiceOver, Claro/Escuro e Movimento Reduzido)

1. **Alternativa de teclado para arrastar e soltar (`hig-drag-and-drop`):**
   - O botão `"Escolher arquivo"` no cabeçalho e a própria área de soltar (`role="button"`, `tabIndex={0}`, acionada por `Enter` ou `Espaço`) garantem que nenhuma ação dependa de arrastar com o mouse.
2. **Renomeação de falantes por teclado e leitor de tela:**
   - Cada chip de falante é um `<button type="button">` com `aria-label="Voz: Pessoa 1. Pressione Enter para mudar o nome."`.
   - Ao ativar, o foco vai direto para o `<input aria-label="Novo nome para Pessoa 1" />`; `Enter` confirma e devolve o foco ao chip atualizado; `Esc` cancela e devolve o foco ao chip.
3. **Regiões vivas (`aria-live`) durante transcrição e reunião:**
   - Na página `Arquivos`, a mudança de fase (`Lendo o áudio` -> `Transcrevendo` -> `Separando vozes` -> `Pronto`) é anunciada em `role="status"` (`aria-live="polite"`), sem anunciar cada ponto percentual individual para não inundar o VoiceOver (a `ProgressBar` atualiza `aria-valuenow` e `aria-valuetext`).
   - Na página `Reunião`, a barra de controle tem `role="status"` para mudanças de estado (`Em reunião`, `Pausada`, `Fechando a reunião`), enquanto a lista de falas ao vivo usa `aria-live="polite"` apenas quando o usuário não está com foco dentro de um campo de edição de nome.
4. **Pílula flutuante (`Pilula.swift`):**
   - Os dois botões do hover (`Ditar` e `Reunião`) e os botões da fase `.reuniao` (`Abrir reunião`, `Pausar a transcrição da reunião`, `Encerrar a reunião`) expõem `.accessibilityLabel` e `.help` explícitos.
   - No modo de movimento reduzido (`accessibilityReduceMotion`), o contorno azul de progresso atualiza o comprimento do arco sem animação elástica e os pontos/barras seguem as regras de `docs/motion.md`.
5. **Contraste (WCAG 2.2 AA em Claro e Escuro):**
   - Os 5 pares de cores de falantes (seção 2.4) usam matizes escurecidos no tema Claro (`#004b99`, `#473299`, `#085e56`, `#7d4000`, `#872048`) e clareados no tema Escuro (`#7cc0ff`, `#c7beff`, `#7ef3e3`, `#ffd080`, `#ffb3d9`), todos com razão de contraste `>= 4,5:1` sobre o fundo da pílula e sobre `--surface-inset`.

---

## 7. Esboços Visuais (ASCII)

### 7.1 Página "Arquivos": Fila em Processamento + Arquivo Pronto com Falantes

```text
+----------------------------------------------------------------------------------------------------+
| (o)(o)(o)                                                                                          |
|           +--------------------------------------------------------------------------------------+ |
| Dita.se   | Arquivos                                                        [ Escolher arquivo ] | |
|           | Transcreva gravações de áudio ou vídeo e separe quem falou em cada trecho.           | |
| [ ] Início|                                                                                      | |
| [ ] Reuni.| +-- DropZone compacta (borda tracejada) -------------------------------------------+ | |
| [*] Arqui.| | [arq] Arraste mais arquivos para cá para pôr na fila   mp3 · m4a · wav · mp4 · mov| | |
| [ ] Insig.| +----------------------------------------------------------------------------------+ | |
| [ ] Dicio.|                                                                                      | |
| [ ] Ajust.| Processando 1 de 2 arquivos                                          [Cancelar fila] | |
|           |                                                                                      | |
|           | +-- Cartão 1: Transcrevendo -------------------------------------------------------+ | |
|           | | [vid] depoimento-parte-2.mp4                        [Transcrevendo · 54%]  [ X ] | | |
|           | |       18 min 40 s · Reconhecendo a fala no Parakeet Ultra                        | | |
|           | |       [=====================>                                                  ] | | |
|           | +----------------------------------------------------------------------------------+ | |
|           |                                                                                      | |
|           | +-- Cartão 2: Pronto (Expandido) --------------------------------------------------+ | |
|           | | [aud] audiencia-18-11.m4a                  [ Copiar ] [ Exportar v ] [ ^ ] [ ... ]| | |
|           | |       Hoje, 10:15 · 42 min 18 s · 3 vozes · 5.420 palavras · pronto em 19 s      | | |
|           | | +------------------------------------------------------------------------------+ | | |
|           | | | Vozes: (*) Pessoa 1 (18) [edit]   (*) Pessoa 2 (14) [edit]   (*) Pessoa 3 (6)| | | |
|           | | |------------------------------------------------------------------------------| | | |
|           | | | (*) Pessoa 1   00:00                                                    [cp] | | | |
|           | | | Bom dia a todos. Vamos abrir a gravação da reunião de alinhamento sobre o    | | | |
|           | | | cronograma de entrega desta semana.                                          | | | |
|           | | |                                                                              | | | |
|           | | | (*) Pessoa 2   00:19                                                    [cp] | | | |
|           | | | Perfeito. Da nossa parte, os testes locais do reconhecedor já terminaram e   | | | |
|           | | | os números bateram com a referência.                                         | | | |
|           | | +------------------------------------------------------------------------------+ | | |
|           | +----------------------------------------------------------------------------------+ | |
| (*) Trans-|                                                                                      | |
| crevendo  | +-- Cartão 3: Formato não suportado -----------------------------------------------+ | |
| arquivo   | | [arq] gravacao-antiga.wma                          [Formato não suportado] [lixo]| | |
| 54%       | |       O macOS não conseguiu abrir este formato. Use mp3, m4a, wav, flac ou mp4.  | | |
+-----------+--------------------------------------------------------------------------------------+ |
```

### 7.2 Página "Reunião": Antes de Começar (com Permissão Pendente) e Durante a Reunião

```text
[A. Antes de começar (com permissão pendente)]
+--------------------------------------------------------------------------------------+
| Reunião                                                          [ Começar reunião ] |
| Transcreve chamadas deste Mac junto com a sua voz e gera um resumo local ao terminar.|
|                                                                                      |
| +-- Aviso de permissão (Inset) ----------------------------------------------------+ |
| | [!] Permita capturar o áudio do Mac                                   [ Permitir ]| |
| |     É o que deixa o Dita.se ouvir as outras pessoas da chamada (Meet, Zoom,      | |
| |     Teams). Nenhuma imagem da tela é gravada e o áudio não sai deste Mac.        | |
| +----------------------------------------------------------------------------------+ |
|                                                                                      |
| +-- Fontes da reunião (Shell) -----------------------------------------------------+ |
| | Fontes de áudio: [ Áudio do Mac + meu microfone (sai como Você) v ] [ok] Pronto  | |
| +----------------------------------------------------------------------------------+ |

[B. Durante a reunião (quase ao vivo)]
+--------------------------------------------------------------------------------------+
| +-- Reunião ao vivo (Shell) -------------------------------------------------------+ |
| | (*) Reunião de hoje, 10:52 [edit]   14:28        [ Pausar ]  [ Encerrar (stop) ] | |
| |     Áudio do Mac + microfone (Você) · [Quase ao vivo · ~2 s de atraso]           | |
| | +------------------------------------------------------------------------------+ | |
| | | Vozes: (*) Você [edit]   (*) Marina [edit]   (*) Pessoa 2 [edit]             | | |
| | |------------------------------------------------------------------------------| | |
| | | (*) Você   13:40                                                             | | |
| | | Então fechamos a entrega da interface hoje e rodamos a suíte logo depois.    | | |
| | |                                                                              | | |
| | | (*) Marina   13:58                                                           | | |
| | | Combinado. Eu vou revisar os arquivos da ponte assim que o build subir.      | | |
| | |                                                                              | | |
| | | (*) Pessoa 2   14:22                                                         | | |
| | | Só confirmando se o formato de legenda srt também entra nesta versão...      | | |
| | +------------------------------------------------------------------------------+ | |
| +----------------------------------------------------------------------------------+ |
+--------------------------------------------------------------------------------------+
```

### 7.3 Página "Reunião": Depois de Encerrar (Resumo Local + Baixa Confiança + Transcrição)

```text
+--------------------------------------------------------------------------------------+
| +-- Reunião concluída (Shell) -----------------------------------------------------+ |
| | Reunião de alinhamento [edit]                    [ Copiar tudo ] [ Exportar v ] [.]|
| | Hoje, 10:52 · 38 min 12 s · 3 vozes (Você, Marina, Pessoa 2) · 4.810 palavras    | |
| | +------------------------------------------------------------------------------+ | |
| | | +-- Resumo (Inset) --------------------------------------------------------+ | | |
| | | | Resumo                                  [Gerado localmente · Qwen]  [cp] | | | |
| | | | * A entrega da interface e dos testes locais ficou confirmada para hoje. | | | |
| | | | * Marina fará a revisão do contrato da ponte após o build.               | | | |
| | | | * A exportação de legendas .srt e .vtt foi incluída no escopo.           | | | |
| | | | Resumo gerado por modelo local a partir da transcrição. Confira abaixo.  | | | |
| | | +--------------------------------------------------------------------------+ | | |
| | |                                                                              | | |
| | | +-- Aviso de baixa confiança (quando aplicável) ---------------------------+ | | |
| | | | (i) Não foi possível separar bem as vozes nesta gravação. Clique no nome | | | |
| | | |     acima de qualquer trecho para indicar quem falou.                    | | | |
| | | +--------------------------------------------------------------------------+ | | |
| | |                                                                              | | |
| | | Vozes: (*) Você (22)  (*) Marina (19)  (*) Pessoa 2 (11)  [Buscar na reunião]| | |
| | |------------------------------------------------------------------------------| | |
| | | (*) Você   00:00                                                        [cp] | | |
| | | Bom dia, pessoal. Vamos repassar os três itens do lançamento...              | | |
| | +------------------------------------------------------------------------------+ | |
| +----------------------------------------------------------------------------------+ |
+--------------------------------------------------------------------------------------+
```

### 7.4 Pílula e Botão Flutuante na Doca (`Ditado/Pilula.swift`)

```text
1. Repouso (40 x 8 pt):
                    ( ====== )

2. Hover sobre a pílula em repouso (176 x 32 pt, cápsula dividida):
         +----------------------------------+
         |  [mic] Ditar  |  [onda] Reunião  |
         +----------------------------------+
          (92 pt, 1 clique) (83 pt, 1 clique)

3. Reunião sendo transcrita (212 x 30 pt):
     +--------------------------------------------+
     | (*) Reunião  |||||  14:28   ( || )  ( [] ) |
     +--------------------------------------------+
       ^ clique abre aba Reunião   Pausar  Encerrar

4. Processando arquivo ou fechando reunião (136 a 148 x 30 pt, contorno azul 2 pt):
            ╔════════════════════════════╗
            ║  [onda] Arquivo · 48%      ║  <- Borda azul preenche no sentido horário (12h -> 12h)
            ╚════════════════════════════╝
```

---

## 8. Matriz de Rastreabilidade e Critérios de Aceite

| ID | Requisito do Pedido / Plano | Local | Critério Observável |
|---|---|---|---|
| `UX-L01` | Entrada de arquivos na lateral + área de soltar + botão + arrastar em qualquer página | `App.tsx` e `paginas/Arquivos.tsx` | Aba `"Arquivos"` na lateral; botão `"Escolher arquivo"` e `DropZone`; arrastar arquivo sobre `Início`, `Dicionário` etc. exibe overlay e leva para `Arquivos` ao soltar. |
| `UX-L02` | Formatos aceitos, fila de vários arquivos e progresso por arquivo | `paginas/Arquivos.tsx` | Aceita `mp3, m4a, wav, aac, flac, ogg, mp4, mov, m4v, webm`; vários arquivos entram em fila com `ProgressBar` individual e status da etapa atual. |
| `UX-L03` | Todos os estados de arquivo cobertos | `paginas/Arquivos.tsx` | Exibe estados `Na fila`, `Lendo o áudio`, `Transcrevendo`, `Separando vozes`, `Pronto`, `Formato não suportado`, `Sem fala detectada` e `Falhou` com textos e ações da seção 2.3. |
| `UX-L04` | Resultado por falante, renomeável, cores discretas, tempos, copiar e exportar | `paginas/Arquivos.tsx` e `Reuniao.tsx` | Chips de vozes (`Pessoa 1, 2…`, `Você`) renomeáveis em linha com atualização global no item; 5 cores semânticas suaves + neutro; carimbos `mm:ss`; copiar e exportar em `.txt`, `.md`, `.srt` e `.vtt`. |
| `UX-L05` | Aba Reunião: antes de começar (fontes e permissão pendente) | `paginas/Reuniao.tsx` | Seleciona `"Áudio do Mac + meu microfone"` ou `"Só o áudio do Mac"`; exibe card de permissão pendente quando faltar áudio do sistema ou microfone. |
| `UX-L06` | Aba Reunião: durante (rolando por falante, tempo, atraso, pausar, encerrar) | `paginas/Reuniao.tsx` | Mostra tempo decorrido, pílula de atraso (`Quase ao vivo · ~2 s` ou `Atraso de ~8 s`), botões `Pausar`/`Retomar` e `Encerrar`, e lista rolando com botão `"Acompanhar ao vivo"` se o usuário rolar para cima. |
| `UX-L07` | Aba Reunião: depois (resumo local rotulado, transcrição, renomear, copiar, exportar, apagar, lista anterior, baixa confiança) | `paginas/Reuniao.tsx` | Resumo no topo com tag `"Gerado localmente · Qwen"` separado da transcrição; aviso `"Não foi possível separar bem as vozes"` quando a confiança for baixa; lista de reuniões anteriores agrupada por dia. |
| `UX-L08` | Botão na doca/pílula: iniciar/encerrar reunião e contorno azul de progresso | `Ditado/Pilula.swift` | No hover em repouso (`176×32 pt`), oferece `"Ditar"` e `"Reunião"` lado a lado (mais menu de clique direito); em reunião (`212×30 pt`), mostra ponto, `"Reunião"`, barras, tempo, pausar e encerrar; contorno azul aparece ao processar arquivo, fechar reunião ou processar ditado com fração conhecida. |

---

## 9. Decisões de Design

1. **Duas abas na lateral (`Reunião` e `Arquivos` logo abaixo de `Início`):** mantém o padrão de página única com largura máxima `52rem` do Dita.se, sem criar subabas aninhadas nem modais pesados.
2. **Exportação incluindo `.srt` e `.vtt` além de `.txt` e `.md`:** como a página `Arquivos` aceita vídeos (`.mp4`, `.mov`, `.m4v`, `.webm`) e o Parakeet já fornece os intervalos de tempo de cada segmento, exportar legendas `.srt` e `.vtt` aproveita o dado existente sem custo extra de interface.
3. **Cápsula dividida no hover da pílula (`Ditar | Reunião`, `176×32 pt`) + menu contextual no clique direito:** preserva o clique único imediato para `"Ditar"` e dá a `"Reunião"` o mesmo acesso de 1 clique, sem esconder a função apenas em menu de botão direito (que violaria a HIG).
4. **Edição de nome de falante em linha e reatribuição por bloco:** clicar no chip de um falante na faixa superior renomeia todas as falas dele de uma vez; quando houver baixa confiança na separação de vozes, o usuário também pode clicar no falante de um bloco específico para reatribuir só aquele trecho.
5. **Pílula durante a reunião:** no estado de reunião ativa vale a versão de `docs/motion.md` (`88×26 pt`, ponto vermelho que acompanha o nível do áudio e o tempo, sem barras; hover expande para `140×30 pt` com `"Abrir"` e encerrar), por ser mais discreta durante a chamada e caber no orçamento de CPU de horas de reunião. Pausar fica na aba Reunião e no menu do clique direito. O restante desta seção (cápsula dividida `"Ditar | Reunião"` no hover em repouso, menu do clique direito e os pontos do contorno azul) continua valendo.
