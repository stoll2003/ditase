# Roteiro de QA do executável instalado no Windows (Dita.se 1.0.2)

Automação independente para validar o executável **instalado** (`Dita.se.exe` via NSIS) pela interface real (WebView2 + diálogo nativo do Windows), sem modificar código de produção.

## 1. Diagnóstico reproduzível da tag `v1.0.1`

**Causa raiz confirmada por inspeção de código (reproduzível pela automação abaixo):**
1. Na abertura do app instalado (`windows/src-tauri/src/main.rs:475`), `motores.preparar_fala()` carrega (ou baixa) o Parakeet em thread separada.
2. A janela principal monta imediatamente e `ui/src/App.tsx:32` chama `useDados("lancamento.estado", {}, ["lancamento"], mesclarLancamento)`.
3. Enquanto o Parakeet ainda carrega, `windows/src-tauri/src/lancamento.rs:484` devolve `"disponivel": self.motores.fala_disponivel()` igual a `false`.
4. Quando o Parakeet entra em `Pronto`, o callback `ao_mudar` em `windows/src-tauri/src/main.rs:320` emite **apenas** `ponte::emitir(&estado, "estado")` e **nunca** `ponte::emitir(&estado, "lancamento")`.
5. Como `App.tsx:32` só assina `["lancamento"]` para recarregar `lancamento.estado`, `lance.disponivel` permanece `false` por toda a sessão:
   - **Arquivos (`ui/src/paginas/Arquivos.tsx:105-107`):** botão `"Escolher arquivo"` e área de arraste ficam desabilitados, exibindo `"A transcrição de arquivos está indisponível nesta versão do motor."`.
   - **Reunião (`ui/src/paginas/Reuniao.tsx:150-152`):** botão `"Começar reunião"` fica desabilitado, exibindo `"A captura de reuniões está indisponível nesta versão do motor."`.
6. **Causa correlata (`windows/src-tauri/src/ponte.rs:264-272`):** `permissao.pedir` para microfone também emite apenas `"estado"` e não `"lancamento"`, impedindo atualização imediata da tela de Reunião quando a permissão muda.

## 2. Como executar em ambiente Windows autorizado

Pré-requisitos na máquina Windows (ou runner `windows-2025` quando autorizado):
- Windows 10/11 x64 com WebView2 Runtime e Node.js 24+.
- Instalador NSIS (`Dita.se-*-setup.exe`) da `v1.0.1` (para diagnóstico) ou do build `1.0.2` corrigido.
- Pasta com modelos já verificados (opcional via `-PastaModelos`, para evitar download de ~700 MB; se omitido, o app baixa).
- Áudio de referência versionado no repositório: `motores/corpus/falantes/conversa-2-vozes.wav` e `conversa-2-vozes.json`.

### Diagnóstico da `v1.0.1` (espera-se detectar os botões desabilitados e salvar screenshots)

```powershell
.\scripts\windows\qa-instalado\executar-qa-instalado.ps1 `
  -Instalador .\caminho\Dita.se-1.0.1-setup.exe `
  -PastaModelos "$env:RUNNER_TEMP\ditase-modelos-cache" `
  -Saida .\qa-saida-v101 `
  -Modo diagnostico
```

### Validação completa no build corrigido (`1.0.2`)

```powershell
.\scripts\windows\qa-instalado\executar-qa-instalado.ps1 `
  -Instalador .\caminho\Dita.se-1.0.2-setup.exe `
  -PastaModelos "$env:RUNNER_TEMP\ditase-modelos-cache" `
  -Saida .\qa-saida-v102 `
  -Modo completo
```

## 3. O que a automação valida

1. **Instalação limpa e isolada:** instala o NSIS silenciosamente em `%LOCALAPPDATA%\Dita.se QA <guid>` (`/S /D=...`), define `DITASE_RAIZ` isolado (nunca toca nos dados do usuário) e `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222`.
2. **Carga do reconhecedor:** aguarda via UI/ponte que o reconhecedor chegue a `pronto`.
3. **Arquivos pela UI:** navega até **Arquivos**, confere que `"Escolher arquivo"` está habilitado, clica no botão, seleciona `conversa-2-vozes.wav` no diálogo nativo do Windows via UI Automation (`selecionar-arquivo-dialogo.ps1`), aguarda o card chegar em `"Pronto"`, lê o texto renderizado na UI e valida contra as palavras conhecidas de `conversa-2-vozes.json`.
4. **Reunião pela UI:** navega até **Reunião**, seleciona `"Só o áudio do computador"`, confere que `"Começar reunião"` está habilitado, inicia a reunião, reproduz `conversa-2-vozes.wav` no dispositivo de saída padrão do Windows (WASAPI loopback), aguarda a reprodução + margem, clica em `"Encerrar"`, aguarda finalizar, abre a reunião em `"Reuniões anteriores"` pelos controles da UI, lê a transcrição exibida e valida contra as palavras conhecidas.
5. **Evidências:** grava screenshots PNG de cada etapa (`01-inicio.png` a `07-reuniao-transcricao.png`) e `relatorio-qa.json` na pasta `-Saida`, encerra o processo e desinstala a cópia de teste em `finally`.

## 4. Limites conhecidos

- **Áudio de sistema (WASAPI loopback) em runner sem placa de som:** `windows-2025` no GitHub Actions não possui dispositivo de saída de áudio por padrão; sem driver virtual (ex.: VB-Cable), `cpal::default_host().default_output_device()` falha com `"nenhuma saída de áudio para capturar"`. O script detecta a ausência de dispositivo de saída via `Win32_SoundDevice`, registra o limite em `relatorio-qa.json` e ainda executa integralmente o diagnóstico de disponibilidade e o fluxo de **Arquivos** pela UI.
- **Execução local no Mac:** este Mac não tem VM Windows nem runner Windows self-hosted autorizado; a execução real do `.exe` instalado depende de máquina Windows autorizada ou disparo de workflow em `windows-2025` pelo Founder.
