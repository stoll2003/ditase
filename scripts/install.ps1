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

    # PowerShell 5.1 ainda pode vir sem TLS 1.2 ligado.
    try {
        [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
    } catch { }

    $tmp = Join-Path ([IO.Path]::GetTempPath()) ("ditase-" + [Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $tmp | Out-Null

    try {
        Write-Host "Dita.se: baixando a versao do canal '$Canal'..."
        $exe = Join-Path $tmp $instalador
        $sumsArq = Join-Path $tmp $somas
        foreach ($par in @(@($instalador, $exe), @($somas, $sumsArq))) {
            $origem = "$base/$($par[0])"
            if (Test-Path -LiteralPath $base) {
                Copy-Item -LiteralPath (Join-Path $base $par[0]) -Destination $par[1]
            } else {
                try {
                    Invoke-WebRequest -Uri $origem -OutFile $par[1] -UseBasicParsing
                } catch {
                    throw "Nao foi possivel baixar $origem. Confira a internet. Se o canal for 'estavel', talvez ainda nao exista versao para Windows. Detalhe: $($_.Exception.Message)"
                }
            }
        }

        Write-Host 'Conferindo o SHA-256...'
        $esperado = $null
        foreach ($linha in Get-Content -LiteralPath $sumsArq) {
            $partes = $linha.Trim() -split '\s+', 2
            if ($partes.Count -eq 2 -and $partes[1].TrimStart('*') -eq $instalador) {
                $esperado = $partes[0].ToLowerInvariant()
                break
            }
        }
        if (-not $esperado) { throw "Hash de $instalador nao encontrado em $somas." }
        $atual = (Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($atual -ne $esperado) {
            throw "Falha na conferencia do SHA-256 (esperado $esperado, calculado $atual). Nada foi instalado."
        }

        Unblock-File -LiteralPath $exe -ErrorAction SilentlyContinue

        # Fecha o Dita.se se estiver aberto, para a instalação poder substituir os arquivos.
        $registro = Get-DitaseRegistro
        if ($registro -and $registro.DisplayIcon) {
            $exeApp = ($registro.DisplayIcon -split ',')[0].Trim('"')
            Get-Process -ErrorAction SilentlyContinue |
                Where-Object { $_.Path -and ($_.Path -ieq $exeApp) } |
                Stop-Process -Force -ErrorAction SilentlyContinue
        }

        Write-Host 'Instalando (sem pedir senha de administrador)...'
        Write-Host 'Se aparecer "O Windows protegeu o computador", clique em "Mais informacoes" e depois em "Executar assim mesmo". O aviso aparece porque o Dita.se e gratuito e nao tem certificado pago; o arquivo ja foi conferido pelo SHA-256.'
        $proc = Start-Process -FilePath $exe -ArgumentList '/S' -Wait -PassThru
        if ($proc.ExitCode -ne 0) { throw "O instalador terminou com erro (codigo $($proc.ExitCode))." }

        $registro = Get-DitaseRegistro
        if (-not $registro -or -not $registro.DisplayIcon) {
            throw 'A instalacao terminou, mas o Dita.se nao foi encontrado no registro do usuario.'
        }
        $exeApp = ($registro.DisplayIcon -split ',')[0].Trim('"')
        if (-not (Test-Path -LiteralPath $exeApp)) { throw "Executavel nao encontrado: $exeApp" }

        $nome = if ($registro.DisplayName) { $registro.DisplayName } else { 'Dita.se' }
        $desktop = [Environment]::GetFolderPath('Desktop')
        $atalho = Join-Path $desktop "$nome.lnk"
        if (-not (Test-Path -LiteralPath $atalho)) {
            $shell = New-Object -ComObject WScript.Shell
            $lnk = $shell.CreateShortcut($atalho)
            $lnk.TargetPath = $exeApp
            $lnk.WorkingDirectory = Split-Path -Parent $exeApp
            $lnk.IconLocation = "$exeApp,0"
            $lnk.Save()
        }

        $versao = if ($registro.DisplayVersion) { " $($registro.DisplayVersion)" } else { '' }
        Write-Host ""
        Write-Host "Pronto: Dita.se$versao instalado em $(Split-Path -Parent $exeApp)."
        Write-Host "Atalho na Area de Trabalho: $atalho"
        Write-Host 'Daqui em diante o app se atualiza sozinho ao abrir.'

        if ($env:DITASE_NAO_ABRIR -ne '1') {
            Start-Process -FilePath $exeApp
        }
    } finally {
        Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue
    }
}

function Get-DitaseRegistro {
    $chave = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*'
    Get-ItemProperty -Path $chave -ErrorAction SilentlyContinue |
        Where-Object { $_.PSObject.Properties['DisplayName'] -and $_.DisplayName -like 'Dita.se*' } |
        Select-Object -First 1
}

Install-Ditase -Canal $Canal
