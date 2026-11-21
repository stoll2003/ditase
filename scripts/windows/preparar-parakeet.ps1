# Baixa e confere os arquivos do modelo Parakeet definidos em motores/src/catalogo.rs
# para uso nos testes de regressão de WER no CI (com cache verificado por SHA-256).
param(
  [string]$Destino,
  [string]$Catalogo
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

$Raiz = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
if (-not $Catalogo) { $Catalogo = Join-Path $Raiz "motores\src\catalogo.rs" }
if (-not $Destino) {
  if ($env:DITASE_MODELOS) {
    $Destino = $env:DITASE_MODELOS
  } elseif ($env:RUNNER_TEMP) {
    $Destino = Join-Path $env:RUNNER_TEMP "ditase-modelos-cache"
  } else {
    $Destino = Join-Path $Raiz "target\modelos-cache"
  }
}

if (-not (Test-Path -LiteralPath $Catalogo -PathType Leaf)) {
  throw "Catálogo não encontrado: $Catalogo"
}

$conteudo = Get-Content -LiteralPath $Catalogo -Raw -Encoding UTF8
$bloco = [regex]::Match(
  $conteudo,
  'pub const PARAKEET:\s*ConjuntoDeModelo\s*=\s*ConjuntoDeModelo\s*\{(?<body>[\s\S]*?)\n\};'
)
if (-not $bloco.Success) {
  throw "Bloco pub const PARAKEET não encontrado em $Catalogo"
}
$corpo = $bloco.Groups['body'].Value

$pastaMatch = [regex]::Match($corpo, 'pasta:\s*"([^"]+)"')
if (-not $pastaMatch.Success) {
  throw "Campo pasta de PARAKEET não encontrado em $Catalogo"
}
$subpasta = $pastaMatch.Groups[1].Value

$padraoArquivo = 'ArquivoRemoto\s*\{\s*arquivo:\s*"([^"]+)",\s*url:\s*"(https://[^"]+)",\s*sha256:\s*"([0-9a-f]{64})",\s*tamanho:\s*([0-9_]+),'
$itens = [regex]::Matches($corpo, $padraoArquivo)
if ($itens.Count -eq 0) {
  throw "Nenhum ArquivoRemoto de PARAKEET encontrado em $Catalogo"
}

$dirParakeet = Join-Path $Destino $subpasta
New-Item -ItemType Directory -Force -Path $dirParakeet | Out-Null

function Baixar-E-Conferir {
  param(
    [Parameter(Mandatory)][string]$Url,
    [Parameter(Mandatory)][string]$ArquivoDestino,
    [Parameter(Mandatory)][string]$Sha256Esperado,
    [Parameter(Mandatory)][long]$TamanhoEsperado
  )

  if (Test-Path -LiteralPath $ArquivoDestino -PathType Leaf) {
    $tamAtual = (Get-Item -LiteralPath $ArquivoDestino).Length
    if ($tamAtual -eq $TamanhoEsperado) {
      $hashAtual = (Get-FileHash -Algorithm SHA256 -LiteralPath $ArquivoDestino).Hash.ToLowerInvariant()
      if ($hashAtual -eq $Sha256Esperado) {
        Write-Host "Cache válido: $(Split-Path -Leaf $ArquivoDestino)"
        return
      }
    }
    Write-Host "Cache inválido ou incompleto descartado: $(Split-Path -Leaf $ArquivoDestino)"
    Remove-Item -Force -LiteralPath $ArquivoDestino
  }

  $parcial = "$ArquivoDestino.part"
  try {
    for ($tentativa = 1; $tentativa -le 3; $tentativa++) {
      try {
        Write-Host "Baixando $Url (tentativa $tentativa/3)..."
        Invoke-WebRequest -Uri $Url -OutFile $parcial -UseBasicParsing -TimeoutSec 300
        break
      } catch {
        if ($tentativa -eq 3) { throw }
        Start-Sleep -Seconds (2 * $tentativa)
      }
    }
    $tam = (Get-Item -LiteralPath $parcial).Length
    if ($tam -ne $TamanhoEsperado) {
      throw "Tamanho divergente para $Url. Esperado: $TamanhoEsperado, obtido: $tam"
    }
    $hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $parcial).Hash.ToLowerInvariant()
    if ($hash -ne $Sha256Esperado) {
      throw "SHA-256 divergente para $Url. Esperado: $Sha256Esperado, obtido: $hash"
    }
    Move-Item -LiteralPath $parcial -Destination $ArquivoDestino
    Write-Host "SHA-256 ok: $(Split-Path -Leaf $ArquivoDestino)"
  } finally {
    Remove-Item -Force -LiteralPath $parcial -ErrorAction SilentlyContinue
  }
}

foreach ($m in $itens) {
  $nome = $m.Groups[1].Value
  $url = $m.Groups[2].Value
  $sha = $m.Groups[3].Value.ToLowerInvariant()
  $tam = [long]($m.Groups[4].Value -replace '_', '')
  Baixar-E-Conferir `
    -Url $url `
    -ArquivoDestino (Join-Path $dirParakeet $nome) `
    -Sha256Esperado $sha `
    -TamanhoEsperado $tam
}

if ($env:GITHUB_ENV) {
  Add-Content -LiteralPath $env:GITHUB_ENV -Value "DITASE_MODELOS=$Destino" -Encoding UTF8
}
Write-Host "Parakeet pronto em $dirParakeet ($($itens.Count) arquivos conferidos por SHA-256)."
