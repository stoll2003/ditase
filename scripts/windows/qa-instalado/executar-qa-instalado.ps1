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
    [int]$PrazoModeloMinutos = 20,
    # Ilha (1.0.2): liga pilulaSempreVisivel no perfil isolado e roda conferir-ilha.ps1 com o app aberto.
    [switch]$ConferirIlha
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
if ($ConferirIlha) {
    # Em repouso a ilha só aparece com "sempre visível"; sem isso a bordinha 92 x 7 não existe para medir.
    '{ "pilulaSempreVisivel": "1" }' | Set-Content -LiteralPath (Join-Path $raizDados 'preferencias.json') -Encoding UTF8
}

$envAnterior = @{
    DITASE_RAIZ = $env:DITASE_RAIZ
    DITASE_PASTA_MODELOS = $env:DITASE_PASTA_MODELOS
    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS
}

$procApp = $null
$politicaAplicada = $false

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
    # Porta CDP pela política do WebView2, só para ditase.exe. O runner roda elevado e o Runtime 150+
    # ignora WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS em processo elevado (run 44875174726: o
    # msedgewebview2 subiu sem a chave). A política só é lida sem a variável, então ela sai do
    # ambiente. Escuta só em 127.0.0.1; a chave é removida no finally.
    Remove-Item Env:\WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS -ErrorAction SilentlyContinue
    $nomeExe = [IO.Path]::GetFileName($Executavel)
    $raizPolitica = if (([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { 'HKLM:' } else { 'HKCU:' }
    $chavePolitica = "$raizPolitica\SOFTWARE\Policies\Microsoft\Edge\WebView2\AdditionalBrowserArguments"
    $chaveCriada = -not (Test-Path $chavePolitica)
    New-Item -Path $chavePolitica -Force | Out-Null
    New-ItemProperty -Path $chavePolitica -Name $nomeExe -Value "--remote-debugging-port=$PortaCdp" -PropertyType String -Force | Out-Null
    $politicaAplicada = $true
    Write-Host "Política WebView2 temporária: $chavePolitica\$nomeExe"

    Write-Host "Iniciando executável instalado: $Executavel (CDP porta $PortaCdp)..."
    $procApp = Start-Process -FilePath $Executavel -PassThru

    $nodeArgs = @(
        $scriptUi,
        "--porta=$PortaCdp",
        "--modo=$Modo",
        "--esperado=$Esperado",
        "--perfil-webview=$(Join-Path $env:LOCALAPPDATA 'se.dita.app')",
        "--prazo-modelo-ms=$($PrazoModeloMinutos * 60000)",
        "--wav=$wav",
        "--ref=$refJson",
        "--saida=$Saida",
        "--dialogo=$scriptDialogo",
        "--tem-audio=$temAudioSistema"
    )
    & node @nodeArgs
    $codigoQa = $LASTEXITCODE

    # Diagnóstico do WebView2 com o app ainda aberto: argumentos efetivos, porta real e escutas.
    $diag = Join-Path $Saida 'diagnostico-webview2.txt'
    & {
        "app vivo: $(-not $procApp.HasExited) pid=$($procApp.Id)"
        "elevado: $(([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator))"
        "política: $chavePolitica\$nomeExe = $((Get-ItemProperty -Path $chavePolitica -Name $nomeExe -ErrorAction SilentlyContinue).$nomeExe)"
        '--- msedgewebview2.exe (browser) ---'
        Get-CimInstance Win32_Process -Filter "Name='msedgewebview2.exe'" -ErrorAction SilentlyContinue |
            Where-Object { $_.CommandLine -notmatch '--type=' } |
            ForEach-Object { "pid=$($_.ProcessId) pai=$($_.ParentProcessId) $($_.CommandLine)" }
        '--- DevToolsActivePort ---'
        Get-ChildItem -Path (Join-Path $env:LOCALAPPDATA 'se.dita.app') -Recurse -Filter DevToolsActivePort -ErrorAction SilentlyContinue |
            ForEach-Object { "$($_.FullName): $((Get-Content -LiteralPath $_.FullName -ErrorAction SilentlyContinue) -join ' ')" }
        '--- escutas TCP em 127.0.0.1/::1 ---'
        Get-NetTCPConnection -State Listen -ErrorAction SilentlyContinue |
            Where-Object { $_.LocalAddress -in '127.0.0.1', '::1', '0.0.0.0', '::' } |
            ForEach-Object { "$($_.LocalAddress):$($_.LocalPort) pid=$($_.OwningProcess) $((Get-Process -Id $_.OwningProcess -ErrorAction SilentlyContinue).ProcessName)" }
        '--- política Edge/WebView2 ---'
        foreach ($k in 'HKLM:\SOFTWARE\Policies\Microsoft\Edge', 'HKLM:\SOFTWARE\Policies\Microsoft\Edge\WebView2', 'HKCU:\SOFTWARE\Policies\Microsoft\Edge\WebView2') {
            if (Test-Path $k) { "$k"; Get-ItemProperty $k | Out-String }
        }
    } *>&1 | Out-File -LiteralPath $diag -Encoding UTF8
    Get-Content -LiteralPath $diag | Write-Host

    $codigoIlha = 0
    if ($ConferirIlha -and -not $procApp.HasExited) {
        Write-Host 'Conferindo a ilha instalada (posição, foco, cantos e escalas)...'
        & powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot 'conferir-ilha.ps1') `
            -ProcessoApp $procApp.Id -Saida (Join-Path $Saida 'ilha') -Executavel $Executavel
        $codigoIlha = $LASTEXITCODE
    } elseif ($ConferirIlha) {
        $codigoIlha = 1
        Write-Host 'App saiu antes da conferência da ilha.'
    }

    # Copia o log do app para diagnóstico junto aos screenshots.
    $logApp = Join-Path $raizDados 'ditado.log'
    if (Test-Path -LiteralPath $logApp -PathType Leaf) {
        Copy-Item -LiteralPath $logApp -Destination (Join-Path $Saida 'ditado.log') -Force
    }

    if ($codigoIlha -ne 0) {
        throw "Conferência da ilha falhou (código $codigoIlha). Veja $Saida\ilha\ilha.json."
    }
    if ($codigoQa -ne 0) {
        throw "Veredito não confere com -Esperado $Esperado (código $codigoQa). Veja $Saida\relatorio-qa.json."
    }
    Write-Host "QA concluído. Evidências em $Saida"
} finally {
    if ($politicaAplicada) {
        Remove-ItemProperty -Path $chavePolitica -Name $nomeExe -ErrorAction SilentlyContinue
        if ($chaveCriada) { Remove-Item -Path $chavePolitica -ErrorAction SilentlyContinue }
    }
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
