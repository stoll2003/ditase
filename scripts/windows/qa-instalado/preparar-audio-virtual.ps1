# Endpoint de saída de áudio virtual para a etapa de Reunião no runner (sem placa de som).
# VB-CABLE pacote 45: o catálogo do driver Windows 10 x64 é assinado pela Microsoft
# ("Microsoft Windows Hardware Compatibility Publisher"). Nada de bcdedit, test signing, política de
# CI, Secure Boot ou TrustedPublisher: se o Windows recusar a assinatura, a etapa só registra o limite.
# Licença: donationware, grátis para uso individual; uso por empresa exige licença (vb-audio.com).
# Nunca falha o job: grava audio-virtual.json e a prova decide pela presença real do endpoint.
param(
    [Parameter(Mandatory = $true)][string]$Saida,
    [int]$TimeoutInstalarSegundos = 180
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$URL = 'https://download.vb-audio.com/Download_CABLE/VBCABLE_Driver_Pack45.zip'
$SHA256 = 'b950e39f01af1d04ea623c8f6d8eb9b6ea5c477c637295fabf20631c85116bfb'
$TAMANHO = 1318877

New-Item -ItemType Directory -Force -Path $Saida | Out-Null
$rel = [ordered]@{ pacote = $URL; sha256 = $SHA256; etapas = @(); instalado = $false; endpoints = @() }
function Etapa($nome, $ok, $detalhe) { $script:rel.etapas += [ordered]@{ nome = $nome; ok = $ok; detalhe = "$detalhe" }; Write-Host "[$nome] ok=$ok $detalhe" }

try {
    $dir = Join-Path $env:RUNNER_TEMP ('vbcable-' + [Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    $zip = Join-Path $dir 'pack.zip'
    Invoke-WebRequest -Uri $URL -OutFile $zip -UseBasicParsing -TimeoutSec 120
    $hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $zip).Hash.ToLowerInvariant()
    $len = (Get-Item -LiteralPath $zip).Length
    if ($hash -ne $SHA256 -or $len -ne $TAMANHO) { throw "pacote divergente: sha256=$hash tamanho=$len" }
    Etapa 'download' $true "sha256 e tamanho conferidos"

    $x = Join-Path $dir 'x'
    Expand-Archive -LiteralPath $zip -DestinationPath $x
    $cat = Get-AuthenticodeSignature -LiteralPath (Join-Path $x 'vbaudio_cable64_win10.cat')
    $setup = Get-AuthenticodeSignature -LiteralPath (Join-Path $x 'VBCABLE_Setup_x64.exe')
    Etapa 'assinatura_catalogo' ($cat.Status -eq 'Valid') "$($cat.Status) $($cat.SignerCertificate.Subject)"
    Etapa 'assinatura_setup' ($setup.Status -eq 'Valid') "$($setup.Status) $($setup.SignerCertificate.Subject)"
    if ($cat.Status -ne 'Valid' -or $cat.SignerCertificate.Subject -notmatch 'Microsoft Windows Hardware Compatibility Publisher') {
        throw 'catálogo do driver sem assinatura Microsoft válida; não instalo'
    }
    if ($setup.Status -ne 'Valid') { throw 'instalador sem assinatura válida; não executo' }

    # Windows Server deixa o áudio desligado. Ligar o serviço não reduz segurança.
    foreach ($svc in 'AudioEndpointBuilder', 'Audiosrv') {
        Set-Service -Name $svc -StartupType Automatic
        Start-Service -Name $svc
    }
    Etapa 'servico_audio' $true ((Get-Service Audiosrv).Status)

    $p = Start-Process -FilePath (Join-Path $x 'VBCABLE_Setup_x64.exe') -ArgumentList '-i', '-h' -WorkingDirectory $x -PassThru
    if (-not $p.WaitForExit($TimeoutInstalarSegundos * 1000)) {
        Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
        throw "instalador não terminou em ${TimeoutInstalarSegundos}s (diálogo oculto aguardando?)"
    }
    Etapa 'instalador' $true "código $($p.ExitCode)"

    $fim = (Get-Date).AddSeconds(60)
    do {
        $eps = @(Get-PnpDevice -Class AudioEndpoint -Status OK -ErrorAction SilentlyContinue | Where-Object { $_.FriendlyName -match 'CABLE' })
        if ($eps.Count) { break }
        Start-Sleep -Seconds 2
    } while ((Get-Date) -lt $fim)
    $rel.endpoints = @($eps | ForEach-Object { $_.FriendlyName })
    $drv = @(Get-PnpDevice -ErrorAction SilentlyContinue | Where-Object { $_.FriendlyName -match 'VB-Audio|CABLE' } | ForEach-Object { "$($_.FriendlyName) [$($_.Status)] $($_.Class)" })
    $rel.dispositivos = $drv
    $rel.instalado = [bool]$eps.Count
    Etapa 'endpoint' $rel.instalado ("endpoints: " + ($rel.endpoints -join '; ') + " | dispositivos: " + ($drv -join '; '))
} catch {
    Etapa 'falha' $false $_.Exception.Message
}
$rel | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $Saida 'audio-virtual.json') -Encoding UTF8
exit 0
