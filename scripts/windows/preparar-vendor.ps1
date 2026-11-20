# Baixa e confere os binários de terceiros exigidos pelo tauri.distribuicao.conf.json
# antes do `tauri build` no Windows (CI e canal de teste).
# Tudo vai para windows/vendor/ (ignorado no git).
# Fontes e hashes fixados conforme motores/LICENCAS.md.
param([switch]$SomenteSherpa)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

$Raiz = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$Vendor = Join-Path $Raiz "windows\vendor"
$Cache = Join-Path $Raiz "target\vendor-cache"
if ($env:RUNNER_TEMP) { $Cache = Join-Path $env:RUNNER_TEMP "ditase-vendor-cache" }

New-Item -ItemType Directory -Force -Path $Cache | Out-Null
if (-not $SomenteSherpa -and (Test-Path -LiteralPath $Vendor)) { Remove-Item -Recurse -Force -LiteralPath $Vendor }
$DirSherpa = Join-Path $Vendor "sherpa"
$DirLlama = Join-Path $Vendor "llama"
$DirVc = Join-Path $Vendor "vcruntime"
$DirAvisos = Join-Path $Vendor "avisos"

function Baixar-E-Conferir {
  param(
    [Parameter(Mandatory)][string]$Url,
    [Parameter(Mandatory)][string]$Destino,
    [Parameter(Mandatory)][string]$Sha256Esperado
  )
  if (Test-Path -LiteralPath $Destino) {
    if ((Get-FileHash -Algorithm SHA256 -LiteralPath $Destino).Hash -eq $Sha256Esperado) {
      Write-Host "Cache válido: $(Split-Path -Leaf $Destino)"
      return
    }
    Remove-Item -Force -LiteralPath $Destino
  }

  $parcial = "$Destino.part"
  try {
    for ($tentativa = 1; $tentativa -le 3; $tentativa++) {
      try {
        Write-Host "Baixando $Url (tentativa $tentativa/3)..."
        Invoke-WebRequest -Uri $Url -OutFile $parcial -UseBasicParsing -TimeoutSec 120
        break
      } catch {
        if ($tentativa -eq 3) { throw }
        Start-Sleep -Seconds (2 * $tentativa)
      }
    }
    $hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $parcial).Hash
    if ($hash -ne $Sha256Esperado) {
      throw "SHA-256 divergente para $Url. Esperado: $Sha256Esperado, obtido: $hash"
    }
    Move-Item -LiteralPath $parcial -Destination $Destino
    Write-Host "SHA-256 ok: $(Split-Path -Leaf $Destino)"
  } finally {
    Remove-Item -Force -LiteralPath $parcial -ErrorAction SilentlyContinue
  }
}

# 1. sherpa-onnx v2.0.1 (DLLs de fala e diarização)
$SherpaTar = Join-Path $Cache "sherpa-onnx-v2.0.1-win-x64-shared-MT-Release-lib.tar.bz2"
Baixar-E-Conferir `
  -Url "https://github.com/k2-fsa/sherpa-onnx/releases/download/v2.0.1/sherpa-onnx-v2.0.1-win-x64-shared-MT-Release-lib.tar.bz2" `
  -Destino $SherpaTar `
  -Sha256Esperado "afa4de3ca4ab0afd53bb4aab40d28ee15608364b549ffc0fe0caf6489bfc39a5"

$Extracao = Join-Path $Raiz "target"
if ($env:RUNNER_TEMP) { $Extracao = $env:RUNNER_TEMP }
$TmpSherpa = Join-Path $Extracao "ditase-extract-sherpa"
if (Test-Path $TmpSherpa) { Remove-Item -Recurse -Force $TmpSherpa }
New-Item -ItemType Directory -Force -Path $TmpSherpa | Out-Null
& tar -xjf $SherpaTar -C $TmpSherpa
if ($LASTEXITCODE -ne 0) { throw "Falha ao extrair $SherpaTar" }

$ImportLib = Get-ChildItem -LiteralPath $TmpSherpa -Recurse -Filter "sherpa-onnx-c-api.lib" | Select-Object -First 1
if (-not $ImportLib) { throw "Biblioteca de link do sherpa-onnx ausente." }
$SherpaLib = $ImportLib.DirectoryName
$DllsSherpa = @("sherpa-onnx-c-api.dll", "onnxruntime.dll", "onnxruntime_providers_shared.dll")
foreach ($dll in $DllsSherpa) {
  if (-not (Test-Path -LiteralPath (Join-Path $SherpaLib $dll) -PathType Leaf)) {
    throw "DLL ausente no diretório de bibliotecas do sherpa-onnx: $dll"
  }
}
if ($env:GITHUB_ENV) {
  Add-Content -LiteralPath $env:GITHUB_ENV -Value "SHERPA_ONNX_LIB_DIR=$SherpaLib" -Encoding UTF8
  # Os testes em target/debug/deps também precisam das DLLs quando o Cargo usa cache.
  Add-Content -LiteralPath $env:GITHUB_PATH -Value $SherpaLib -Encoding UTF8
}
if ($SomenteSherpa) { return }

New-Item -ItemType Directory -Force -Path $DirSherpa, $DirLlama, $DirVc, $DirAvisos | Out-Null
foreach ($dll in $DllsSherpa) {
  Copy-Item -LiteralPath (Join-Path $SherpaLib $dll) -Destination (Join-Path $DirSherpa $dll)
}

$AvisoOrt = Get-ChildItem -Path $TmpSherpa -Recurse -Filter "ThirdPartyNotices.txt" | Select-Object -First 1
if ($AvisoOrt) {
  Copy-Item -Path $AvisoOrt.FullName -Destination (Join-Path $DirAvisos "ThirdPartyNotices.txt") -Force
} else {
  # O pacote do sherpa-onnx não traz os avisos do ONNX Runtime (1.28.2). Cópia versionada,
  # extraída do pacote NuGet oficial Microsoft.ML.OnnxRuntime 1.28.0
  # (SHA-256 do .nupkg 769d1d3ea8ab6cd69f737c9dd4d4462aa4ad0ccfa106eaf506efc40d7bead5db).
  Copy-Item -Path (Join-Path $Raiz "windows\avisos\onnxruntime-ThirdPartyNotices.txt") -Destination (Join-Path $DirAvisos "ThirdPartyNotices.txt") -Force
}

# 2. llama.cpp b12359 CPU x64 (limpeza e resumo local)
$LlamaZip = Join-Path $Cache "llama-b12359-bin-win-cpu-x64.zip"
Baixar-E-Conferir `
  -Url "https://github.com/ggml-org/llama.cpp/releases/download/b12359/llama-b12359-bin-win-cpu-x64.zip" `
  -Destino $LlamaZip `
  -Sha256Esperado "837c1655e87df3b66989f4be33a792a1399c785bf259038dc75c900096683e69"

$TmpLlama = Join-Path $Extracao "ditase-extract-llama"
if (Test-Path $TmpLlama) { Remove-Item -Recurse -Force $TmpLlama }
Expand-Archive -Path $LlamaZip -DestinationPath $TmpLlama -Force

$ArquivosLlama = @(
  "llama-server.exe",
  "llama-server-impl.dll",
  "llama-common.dll",
  "llama.dll",
  "mtmd.dll",
  "ggml.dll",
  "ggml-base.dll",
  "ggml-rpc.dll",
  "libomp.dll",
  "LICENSE-LLVM-OpenMP"
)
foreach ($nome in $ArquivosLlama) {
  $item = Get-ChildItem -Path $TmpLlama -Recurse -Filter $nome | Select-Object -First 1
  if (-not $item) { throw "Arquivo ausente no pacote llama.cpp: $nome" }
  Copy-Item -Path $item.FullName -Destination (Join-Path $DirLlama $nome) -Force
}
$CpuDlls = Get-ChildItem -Path $TmpLlama -Recurse -Filter "ggml-cpu-*.dll"
if (-not $CpuDlls) { throw "Nenhuma DLL ggml-cpu-*.dll encontrada no pacote llama.cpp" }
foreach ($cpu in $CpuDlls) {
  Copy-Item -Path $cpu.FullName -Destination (Join-Path $DirLlama $cpu.Name) -Force
}

# 3. Runtime do Visual C++ (Redist do Visual Studio instalado no runner)
$PastasCrt = Get-ChildItem -Path "C:\Program Files\Microsoft Visual Studio\*\*\VC\Redist\MSVC\*\x64\Microsoft.VC*.CRT" -Directory -ErrorAction SilentlyContinue | Sort-Object FullName -Descending
$PastaCrt = $PastasCrt | Select-Object -First 1
if (-not $PastaCrt) {
  throw "Pasta Redist do Visual C++ não encontrada em C:\Program Files\Microsoft Visual Studio"
}
foreach ($dll in @("MSVCP140.dll", "VCRUNTIME140.dll", "VCRUNTIME140_1.dll")) {
  $origem = Join-Path $PastaCrt.FullName $dll
  if (-not (Test-Path $origem)) { throw "DLL do Visual C++ ausente em $($PastaCrt.FullName): $dll" }
  $origemCrt = Get-Item -LiteralPath $origem
  $hashCrt = (Get-FileHash -LiteralPath $origem -Algorithm SHA256).Hash
  Write-Host "Visual C++: $dll $($origemCrt.VersionInfo.FileVersion) SHA-256 $hashCrt"
  Copy-Item -LiteralPath $origem -Destination (Join-Path $DirVc $dll) -Force
}

# 4. Conferência final
$Obrigatorios = @(
  "sherpa\sherpa-onnx-c-api.dll",
  "sherpa\onnxruntime.dll",
  "sherpa\onnxruntime_providers_shared.dll",
  "llama\llama-server.exe",
  "llama\libomp.dll",
  "llama\LICENSE-LLVM-OpenMP",
  "vcruntime\MSVCP140.dll",
  "vcruntime\VCRUNTIME140.dll",
  "vcruntime\VCRUNTIME140_1.dll",
  "avisos\ThirdPartyNotices.txt"
)
foreach ($rel in $Obrigatorios) {
  $p = Join-Path $Vendor $rel
  if (-not (Test-Path $p)) { throw "Artefato obrigatório faltando em windows/vendor: $rel" }
}
Write-Host "windows/vendor pronto: sherpa (3 DLLs), llama ($($ArquivosLlama.Count + $CpuDlls.Count) arquivos), vcruntime (3 DLLs), avisos (1 arquivo)."
