param(
    [Parameter(Mandatory = $false)]
    [string]$Instalador,
    [Parameter(Mandatory = $false)]
    [string]$Executavel,
    [ValidateSet('defeito', 'corrigido')]
    [string]$Esperado = 'corrigido',
    [string]$Saida = (Join-Path $env:TEMP ("ditase-qa-instalado-" + (Get-Date -Format 'yyyyMMdd-HHmmss'))),
    [ValidateSet('diagnostico', 'completo')]
    [string]$Modo = 'completo',
    [int]$PortaCdp = 9222,
    [int]$PrazoModeloMinutos = 20
)

$ErrorActionPreference = 'Stop'

if (-not $Instalador -and -not $Executavel) {
    throw 'Informe -Instalador (caminho do *-setup.exe) ou -Executavel (caminho do Dita.se.exe já instalado).'
}

$raizRepo = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..')).Path
$wav = Join-Path $raizRepo 'motores\corpus\falantes\conversa-2-vozes.wav'
$refJson = Join-Path $raizRepo 'motores\corpus\falantes\conversa-2-vozes.json'
$scriptUi = Join-Path $PSScriptRoot 'prova-ui.mjs'
$scriptDialogo = Join-Path $PSScriptRoot 'selecionar-arquivo-dialogo.ps1'

foreach ($f in @($wav, $refJson, $scriptUi, $scriptDialogo)) {
    if (-not (Test-Path -LiteralPath $f -PathType Leaf)) {
        throw "Arquivo necessário ausente: $f"
    }
}

New-Item -ItemType Directory -Force -Path $Saida | Out-Null
$Saida = (Resolve-Path $Saida).Path

$chaveUninstall = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*'
function Registro-DitaSe {
    Get-ItemProperty -Path $chaveUninstall -ErrorAction SilentlyContinue |
        Where-Object { $_.PSObject.Properties['DisplayName'] -and $_.DisplayName -eq 'Dita.se' } |
        Select-Object -First 1
}

$instalouAqui = $false
$destino = $null
$raizDados = Join-Path ([IO.Path]::GetTempPath()) ("ditase-qa-dados-" + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path $raizDados | Out-Null
# Sempre vazia: fala_disponivel() olha o disco, então qualquer modelo pré-carregado esconderia o
# defeito da v1.0.1. O próprio app baixa o Parakeet nesta pasta, na mesma sessão observada.
$pastaModelos = Join-Path $raizDados 'Modelos'
New-Item -ItemType Directory -Force -Path $pastaModelos | Out-Null
if (@(Get-ChildItem -LiteralPath $pastaModelos -Force).Count -ne 0) { throw "Pasta de modelos não está vazia: $pastaModelos" }

$envAnterior = @{
    DITASE_RAIZ = $env:DITASE_RAIZ
    DITASE_PASTA_MODELOS = $env:DITASE_PASTA_MODELOS
    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS
}

$procApp = $null

try {
    if ($Instalador) {
        if (-not (Test-Path -LiteralPath $Instalador -PathType Leaf)) {
            throw "Instalador não encontrado: $Instalador"
        }
        if (Registro-DitaSe) {
            throw 'Já existe uma instalação de Dita.se neste usuário. Execute em usuário limpo ou passe -Executavel.'
        }
        $destino = Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) ("Dita.se QA " + [Guid]::NewGuid().ToString('N'))
        Write-Host "Instalando $Instalador em $destino..."
        $instalouAqui = $true
        $pInst = Start-Process -FilePath $Instalador -ArgumentList "/S /D=$destino" -Wait -PassThru
        if ($pInst.ExitCode -ne 0) {
            throw "Instalação NSIS falhou com código $($pInst.ExitCode)."
        }
        $reg = Registro-DitaSe
        if (-not $reg) { throw 'Registro HKCU do Dita.se não encontrado após instalação.' }
        $Executavel = ($reg.DisplayIcon -replace ',-?\d+$').Trim('"')
        if (-not (Test-Path -LiteralPath $Executavel -PathType Leaf)) {
            throw "Executável instalado ausente: $Executavel"
        }
        Write-Host "Instalado com sucesso: $Executavel (versão $($reg.DisplayVersion))"
    } elseif (-not (Test-Path -LiteralPath $Executavel -PathType Leaf)) {
        throw "Executável não encontrado: $Executavel"
    }

    # Reunião precisa de um endpoint de SAÍDA ativo (WASAPI loopback do dispositivo padrão) e do
    # serviço de áudio rodando. Win32_SoundDevice sozinho não garante endpoint.
    $audiosrv = Get-Service -Name Audiosrv -ErrorAction SilentlyContinue
    $endpoints = @(Get-PnpDevice -Class AudioEndpoint -Status OK -ErrorAction SilentlyContinue)
    $temAudioSistema = if ($audiosrv -and $audiosrv.Status -eq 'Running' -and $endpoints.Count -gt 0) { '1' } else { '0' }
    [ordered]@{
        audiosrv = if ($audiosrv) { "$($audiosrv.Status)" } else { 'ausente' }
        endpoints = @($endpoints | ForEach-Object { $_.FriendlyName })
        usadoNaProva = ($temAudioSistema -eq '1')
    } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $Saida 'audio-ambiente.json') -Encoding UTF8
    Write-Host "Audiosrv: $($audiosrv.Status); endpoints OK: $($endpoints.Count)"

    $env:DITASE_RAIZ = $raizDados
    $env:DITASE_PASTA_MODELOS = $pastaModelos
    $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "--remote-debugging-port=$PortaCdp"

    Write-Host "Iniciando executável instalado: $Executavel (CDP porta $PortaCdp)..."
    $procApp = Start-Process -FilePath $Executavel -PassThru

    $nodeArgs = @(
        $scriptUi,
        "--porta=$PortaCdp",
        "--modo=$Modo",
        "--esperado=$Esperado",
        "--prazo-modelo-ms=$($PrazoModeloMinutos * 60000)",
        "--wav=$wav",
        "--ref=$refJson",
        "--saida=$Saida",
        "--dialogo=$scriptDialogo",
        "--tem-audio=$temAudioSistema"
    )
    & node @nodeArgs
    $codigoQa = $LASTEXITCODE

    # Copia o log do app para diagnóstico junto aos screenshots.
    $logApp = Join-Path $raizDados 'ditado.log'
    if (Test-Path -LiteralPath $logApp -PathType Leaf) {
        Copy-Item -LiteralPath $logApp -Destination (Join-Path $Saida 'ditado.log') -Force
    }

    if ($codigoQa -ne 0) {
        throw "Veredito não confere com -Esperado $Esperado (código $codigoQa). Veja $Saida\relatorio-qa.json."
    }
    Write-Host "QA concluído. Evidências em $Saida"
} finally {
    if ($procApp -and -not $procApp.HasExited) {
        Write-Host 'Encerrando Dita.se...'
        try { $procApp.CloseMainWindow() | Out-Null } catch {}
        Start-Sleep -Seconds 2
        if (-not $procApp.HasExited) {
            Stop-Process -Id $procApp.Id -Force -ErrorAction SilentlyContinue
        }
    }
    if ($instalouAqui) {
        $reg = Registro-DitaSe
        if ($reg -and $reg.UninstallString -match '^"([^"]+)"\s*$') {
            $desinstalar = $Matches[1]
            Write-Host "Desinstalando cópia de teste ($desinstalar)..."
            Start-Process -FilePath $desinstalar -ArgumentList '/S' -Wait -ErrorAction SilentlyContinue | Out-Null
        }
    }
    foreach ($nome in $envAnterior.Keys) {
        [Environment]::SetEnvironmentVariable($nome, $envAnterior[$nome], 'Process')
    }
    Remove-Item -LiteralPath $raizDados -Recurse -Force -ErrorAction SilentlyContinue
}
