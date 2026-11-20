# Baixa e confere os binários de terceiros exigidos pelo tauri.distribuicao.conf.json
# antes do `tauri build` no Windows (CI e canal de teste).
# Tudo vai para windows/vendor/ (ignorado no git).
# Fontes e hashes fixados conforme motores/LICENCAS.md.
$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

$Raiz = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$Vendor = Join-Path $Raiz "windows\vendor"
$Cache = Join-Path $Raiz "target\vendor-cache"

New-Item -ItemType Directory -Force -Path $Cache | Out-Null
if (Test-Path $Vendor) { Remove-Item -Recurse -Force $Vendor }
$DirSherpa = Join-Path $Vendor "sherpa"
$DirLlama = Join-Path $Vendor "llama"
$DirVc = Join-Path $Vendor "vcruntime"
$DirAvisos = Join-Path $Vendor "avisos"
New-Item -ItemType Directory -Force -Path $DirSherpa, $DirLlama, $DirVc, $DirAvisos | Out-Null

function Baixar-E-Conferir {
  param(
    [Parameter(Mandatory)][string]$Url,
    [Parameter(Mandatory)][string]$Destino,
    [Parameter(Mandatory)][string]$Sha256Esperado
  )
  if (Test-Path $Destino) {
    $atual = (Get-FileHash -Algorithm SHA256 -Path $Destino).Hash.ToLowerInvariant()
    if ($atual -eq $Sha256Esperado.ToLowerInvariant()) {
      Write-Host "Cache válido: $(Split-Path -Leaf $Destino)"
      return
    }
    Remove-Item -Force $Destino
  }
  Write-Host "Baixando $Url ..."
  Invoke-WebRequest -Uri $Url -OutFile $Destino -UseBasicParsing
  $hash = (Get-FileHash -Algorithm SHA256 -Path $Destino).Hash.ToLowerInvariant()
  if ($hash -ne $Sha256Esperado.ToLowerInvariant()) {
    Remove-Item -Force $Destino
    throw "SHA-256 divergente para $Url. Esperado: $Sha256Esperado, obtido: $hash"
  }
  Write-Host "SHA-256 ok: $(Split-Path -Leaf $Destino)"
}

# 1. sherpa-onnx v2.0.1 (DLLs de fala e diarização)
$SherpaTar = Join-Path $Cache "sherpa-onnx-v2.0.1-win-x64-shared-MT-Release-lib.tar.bz2"
Baixar-E-Conferir `
  -Url "https://github.com/k2-fsa/sherpa-onnx/releases/download/v2.0.1/sherpa-onnx-v2.0.1-win-x64-shared-MT-Release-lib.tar.bz2" `
  -Destino $SherpaTar `
  -Sha256Esperado "afa4de3ca4ab0afd53bb4aab40d28ee15608364b549ffc0fe0caf6489bfc39a5"

$TmpSherpa = Join-Path $Cache "extract-sherpa"
if (Test-Path $TmpSherpa) { Remove-Item -Recurse -Force $TmpSherpa }
New-Item -ItemType Directory -Force -Path $TmpSherpa | Out-Null
& tar -xjf $SherpaTar -C $TmpSherpa
if ($LASTEXITCODE -ne 0) { throw "Falha ao extrair $SherpaTar" }

foreach ($dll in @("sherpa-onnx-c-api.dll", "onnxruntime.dll", "onnxruntime_providers_shared.dll")) {
  $encontrado = Get-ChildItem -Path $TmpSherpa -Recurse -Filter $dll | Select-Object -First 1
  if (-not $encontrado) { throw "DLL ausente no pacote sherpa-onnx: $dll" }
  Copy-Item -Path $encontrado.FullName -Destination (Join-Path $DirSherpa $dll) -Force
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

$TmpLlama = Join-Path $Cache "extract-llama"
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
  Copy-Item -Path $origem -Destination (Join-Path $DirVc $dll) -Force
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
