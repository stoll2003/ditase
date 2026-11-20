# Instala o Dita.se no Windows 11 (x64) para o usuário atual, sem administrador.
#
# Uso (PowerShell):
#   irm https://raw.githubusercontent.com/stoll2003/ditase/windows/scripts/install.ps1 | iex
#
# Canal: teste (padrão nesta fase) ou estavel.
#   $env:DITASE_CANAL = 'estavel'; irm https://raw.githubusercontent.com/stoll2003/ditase/windows/scripts/install.ps1 | iex
#   ou, salvo em arquivo: .\install.ps1 -Canal estavel
#
# O que faz: baixa o instalador e o SHA256SUMS.txt do canal, confere o SHA-256,
# instala em silêncio por usuário (%LOCALAPPDATA%), garante o atalho na Área de
# Trabalho e abre o app. Depois disso o próprio app se atualiza ao abrir.
#
# Proxy autenticado: DITASE_PROXY (URL sem usuário/senha; usa o login do Windows).
# Para desligar a abertura automática: DITASE_NAO_ABRIR=1.
# Variáveis para testes: DITASE_REPO, DITASE_BASE_URL (URL ou pasta local),
# DITASE_NAO_ABRIR=1 (não abre o app no fim).

param(
    [ValidateSet('teste', 'estavel')]
    [string]$Canal = $(if ($env:DITASE_CANAL) { $env:DITASE_CANAL } else { 'teste' })
)

function Install-Ditase {
    param([string]$Canal)

    # Só no escopo da função: não altera a sessão de quem rodou o irm | iex.
    $ErrorActionPreference = 'Stop'
    $ProgressPreference = 'SilentlyContinue'

    $repo = if ($env:DITASE_REPO) { $env:DITASE_REPO } else { 'stoll2003/ditase' }
    $instalador = 'Dita.se-windows-x64-setup.exe'
    $somas = 'SHA256SUMS.txt'

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

    # O PowerShell 5.1 usa ServicePointManager; restaure o valor ao voltar do irm | iex.
    $tlsAnterior = [Net.ServicePointManager]::SecurityProtocol

    $tmp = Join-Path ([IO.Path]::GetTempPath()) ("ditase-" + [Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $tmp | Out-Null

    try {
        if ($PSVersionTable.PSEdition -eq 'Desktop') {
            [Net.ServicePointManager]::SecurityProtocol = $tlsAnterior -bor [Net.SecurityProtocolType]::Tls12
        }
        Write-Host "Dita.se: baixando a versão do canal '$Canal'..."
        $exe = Join-Path $tmp $instalador
        $sumsArq = Join-Path $tmp $somas

        if (Test-Path -LiteralPath $base -PathType Container) {
            Copy-Item -LiteralPath (Join-Path $base $somas) -Destination $sumsArq
            Copy-Item -LiteralPath (Join-Path $base $instalador) -Destination $exe
        } else {
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
            Baixar-Ditase "$base/$somas" $sumsArq @proxy
            Baixar-Ditase "$base/$instalador" $exe @proxy
        }

        Write-Host 'Conferindo o SHA-256...'
        $esperado = $null
        foreach ($linha in Get-Content -LiteralPath $sumsArq) {
            $partes = $linha.Trim() -split '\s+', 2
            if ($partes.Count -eq 2 -and $partes[1].TrimStart('*') -eq $instalador) {
                if ($esperado -or $partes[0] -notmatch '\A[0-9a-f]{64}\z') {
                    throw "SHA-256 inválido ou repetido para $instalador em $somas."
                }
                $esperado = $partes[0].ToLowerInvariant()
            }
        }
        if (-not $esperado) { throw "SHA-256 de $instalador não encontrado em $somas." }
        $atual = (Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($atual -ne $esperado) {
            throw "Falha na conferência do SHA-256 (esperado $esperado, calculado $atual). Nada foi instalado."
        }

        # Fecha o Dita.se se estiver aberto, para a instalação poder substituir os arquivos.
        $registro = Get-DitaseRegistro
        if ($registro -and $registro.DisplayIcon) {
            $exeApp = ($registro.DisplayIcon -replace ',-?\d+$').Trim('"')
            Get-Process -ErrorAction SilentlyContinue |
                Where-Object { $_.Path -and ($_.Path -ieq $exeApp) } |
                Stop-Process -Force -ErrorAction SilentlyContinue
        }

        Write-Host 'Instalando (sem pedir senha de administrador)...'
        $proc = Start-Process -FilePath $exe -ArgumentList '/S' -Wait -PassThru
        if ($proc.ExitCode -ne 0) { throw "O instalador terminou com erro (código $($proc.ExitCode))." }

        $registro = Get-DitaseRegistro
        if (-not $registro -or -not $registro.DisplayIcon) {
            throw 'A instalação terminou, mas o Dita.se não foi encontrado no registro do usuario.'
        }
        $exeApp = ($registro.DisplayIcon -replace ',-?\d+$').Trim('"')
        if (-not (Test-Path -LiteralPath $exeApp)) { throw "Executável não encontrado: $exeApp" }

        $nome = if ($registro.DisplayName) { $registro.DisplayName } else { 'Dita.se' }
        $desktop = [Environment]::GetFolderPath('Desktop')
        $atalho = Join-Path $desktop "$nome.lnk"
        if (-not (Test-Path -LiteralPath $atalho -PathType Leaf)) {
            Write-Warning "Atalho ausente na Área de Trabalho. Abra o Dita.se pelo menu Iniciar."
        }

        $versao = if ($registro.DisplayVersion) { " $($registro.DisplayVersion)" } else { '' }
        Write-Host ""
        Write-Host "Pronto: Dita.se$versao instalado em $(Split-Path -Parent $exeApp)."
        Write-Host "Atalho na Área de Trabalho: $atalho"
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

Install-Ditase -Canal $Canal
