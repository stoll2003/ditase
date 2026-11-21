# Auxiliar de instalação para uso local ou em CI. Para o primeiro teste no PC,
# siga scripts/windows/LEIA-ME-TESTE.md e confira o SHA-256 enviado separadamente.
# DITASE_PROXY: URL do proxy sem usuário/senha; usa o login do Windows.
# DITASE_BASE_URL: pasta local ou URL HTTPS de um espelho autorizado.
# DITASE_NAO_ABRIR=1: não abre o aplicativo após instalar.

param(
    [ValidateSet('teste', 'estavel')]
    [string]$Canal = $(if ($env:DITASE_CANAL) { $env:DITASE_CANAL } else { 'estavel' }),
    [ValidatePattern('\A(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)(?:-teste\.[1-9][0-9]*)?\z')]
    [string]$Versao,
    [Parameter(Mandatory)]
    [ValidatePattern('\A[0-9a-fA-F]{64}\z')]
    [string]$Sha256
)

function Install-Ditase {
    param([string]$Canal, [string]$Versao, [string]$Sha256)

    # Limita as preferências de erro e progresso ao escopo da instalação.
    $ErrorActionPreference = 'Stop'
    $ProgressPreference = 'SilentlyContinue'

    $repo = if ($env:DITASE_REPO) { $env:DITASE_REPO } else { 'stoll2003/ditase' }
    if ($Versao) {
        $instalador = "Dita.se-windows-x64-$Versao-setup.exe"
    } else {
        $instalador = 'Dita.se-windows-x64-setup.exe'
    }

    if ($env:DITASE_BASE_URL) {
        $base = $env:DITASE_BASE_URL.TrimEnd('/', '\')
    } elseif ($Canal -eq 'estavel') {
        $base = "https://github.com/$repo/releases/latest/download"
    } else {
        $base = "https://github.com/$repo/releases/download/windows-teste"
    }

    if (-not [Environment]::Is64BitOperatingSystem) {
        throw 'O Dita.se para Windows precisa de um Windows 64 bits.'
    }

    # O PowerShell 5.1 usa ServicePointManager; restaure o valor ao sair.
    $tlsAnterior = [Net.ServicePointManager]::SecurityProtocol

    $tmp = Join-Path ([IO.Path]::GetTempPath()) ("ditase-" + [Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $tmp | Out-Null

    try {
        if ($PSVersionTable.PSEdition -eq 'Desktop') {
            [Net.ServicePointManager]::SecurityProtocol = $tlsAnterior -bor [Net.SecurityProtocolType]::Tls12
        }
        Write-Host "Dita.se: baixando a versão do canal '$Canal'..."
        $exe = Join-Path $tmp $instalador

        if (Test-Path -LiteralPath $base -PathType Container) {
            Copy-Item -LiteralPath (Join-Path $base $instalador) -Destination $exe
        } else {
            if ($Canal -eq 'teste' -and $Versao -notmatch '-teste\.') {
                throw 'Informe a versão de teste em -Versao, como 0.2.0-teste.123.'
            }
            $origem = "$base/$instalador" -as [uri]
            if (-not $origem -or -not $origem.IsAbsoluteUri -or $origem.Scheme -ne 'https' -or $origem.UserInfo) {
                throw 'O endereço do instalador deve usar HTTPS e não pode conter usuário e senha.'
            }
            $proxy = @{}
            if ($env:DITASE_PROXY) {
                $urlProxy = $env:DITASE_PROXY -as [uri]
                if (-not $urlProxy -or -not $urlProxy.IsAbsoluteUri -or
                    $urlProxy.Scheme -notin @('http', 'https') -or $urlProxy.UserInfo) {
                    throw 'DITASE_PROXY deve ser uma URL HTTP ou HTTPS sem usuário e senha.'
                }
                $proxy = @{ Proxy = $urlProxy; ProxyUseDefaultCredentials = $true }
            } elseif ($PSVersionTable.PSEdition -eq 'Desktop') {
                # No Windows PowerShell, use o proxy do sistema, inclusive o definido por PAC.
                $destino = [uri]$base
                $proxySistema = [Net.WebRequest]::DefaultWebProxy
                if ($proxySistema -and -not $proxySistema.IsBypassed($destino)) {
                    $proxy = @{ Proxy = $proxySistema.GetProxy($destino); ProxyUseDefaultCredentials = $true }
                }
            }
            Baixar-Ditase $origem.AbsoluteUri $exe @proxy
            # Mantém o arquivo sujeito às verificações do Windows para downloads.
            Set-Content -LiteralPath $exe -Stream Zone.Identifier -Encoding Ascii -Value @('[ZoneTransfer]', 'ZoneId=3')
        }

        Write-Host 'Conferindo o SHA-256...'
        $atual = (Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($atual -ne $Sha256) {
            throw "Falha na conferência do SHA-256 (esperado $Sha256, calculado $atual). Nada foi instalado."
        }

        $registro = Get-DitaseRegistro
        if ($registro -and $registro.DisplayIcon) {
            $exeApp = ($registro.DisplayIcon -replace ',-?\d+$').Trim('"')
            $aberto = Get-Process -ErrorAction SilentlyContinue |
                Where-Object { $_.Path -and ($_.Path -ieq $exeApp) }
            if ($aberto) { throw 'Encerre o Dita.se pelo ícone da bandeja antes de reinstalar.' }
        }

        Write-Host 'Instalando (sem pedir senha de administrador)...'
        $proc = Start-Process -FilePath $exe -ArgumentList '/S' -Wait -PassThru
        if ($proc.ExitCode -ne 0) { throw "O instalador terminou com erro (código $($proc.ExitCode))." }

        $registro = Get-DitaseRegistro
        if (-not $registro -or -not $registro.DisplayIcon) {
            throw 'A instalação terminou, mas o Dita.se não foi encontrado no registro do usuário.'
        }
        $exeApp = ($registro.DisplayIcon -replace ',-?\d+$').Trim('"')
        if (-not (Test-Path -LiteralPath $exeApp)) { throw "Executável não encontrado: $exeApp" }

        $nome = if ($registro.DisplayName) { $registro.DisplayName } else { 'Dita.se' }
        $desktop = [Environment]::GetFolderPath('Desktop')
        $atalho = Join-Path $desktop "$nome.lnk"
        if (-not (Test-Path -LiteralPath $atalho -PathType Leaf)) {
            Write-Warning "Atalho ausente na Área de Trabalho. Abra o Dita.se pelo menu Iniciar."
        }

        $versaoInstalada = if ($registro.DisplayVersion) { " $($registro.DisplayVersion)" } else { '' }
        Write-Host ""
        Write-Host "Pronto: Dita.se$versaoInstalada instalado em $(Split-Path -Parent $exeApp)."
        if (Test-Path -LiteralPath $atalho -PathType Leaf) { Write-Host "Atalho na Área de Trabalho: $atalho" }
        Write-Host 'O app verifica atualizações ao abrir e pede confirmação antes de instalar.'

        if ($env:DITASE_NAO_ABRIR -ne '1') {
            Start-Process -FilePath $exeApp
        }
    } finally {
        [Net.ServicePointManager]::SecurityProtocol = $tlsAnterior
        Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue
    }
}

function Baixar-Ditase {
    param([string]$Url, [string]$Destino, [uri]$Proxy, [switch]$ProxyUseDefaultCredentials)

    for ($tentativa = 1; $tentativa -le 3; $tentativa++) {
        try {
            $opcoes = @{ Uri = $Url; OutFile = $Destino; UseBasicParsing = $true; TimeoutSec = 180 }
            if ($Proxy) {
                $opcoes.Proxy = $Proxy
                $opcoes.ProxyUseDefaultCredentials = $ProxyUseDefaultCredentials
            }
            Invoke-WebRequest @opcoes
            return
        } catch {
            $status = $_.Exception.Response
            if ($status -and [int]$status.StatusCode -eq 407) {
                throw 'O proxy exige autenticação. Defina DITASE_PROXY com a URL fornecida pela sua TI e tente de novo.'
            }
            if ($tentativa -eq 3 -or ($status -and [int]$status.StatusCode -eq 404)) {
                throw "Não foi possível baixar $Url. Confira a conexão e se o canal está disponível. Detalhe: $($_.Exception.Message)"
            }
            Start-Sleep -Seconds (2 * $tentativa)
        }
    }
}

function Get-DitaseRegistro {
    $chave = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*'
    Get-ItemProperty -Path $chave -ErrorAction SilentlyContinue |
        Where-Object { $_.PSObject.Properties['DisplayName'] -and $_.DisplayName -like 'Dita.se*' } |
        Select-Object -First 1
}

Install-Ditase -Canal $Canal -Versao $Versao -Sha256 $Sha256
