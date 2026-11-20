param(
    [ValidateSet('sintaxe', 'instalacao')]
    [string]$Modo = 'sintaxe',
    [string]$Instalador,
    [string]$VersaoEsperada
)

$ErrorActionPreference = 'Stop'
$script = Join-Path $PSScriptRoot '..\install.ps1' | Resolve-Path
$fonte = Get-Content -LiteralPath $script -Raw -Encoding UTF8
$instalacaoIniciada = $false
$chave = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*'

function Registro-Teste {
    Get-ItemProperty -Path $chave -ErrorAction SilentlyContinue |
        Where-Object { $_.PSObject.Properties['DisplayName'] -and $_.DisplayName -eq 'Dita.se' } |
        Select-Object -First 1
}

function Conferir-Instalacao {
    $reg = Registro-Teste
    if (-not $reg -or -not $reg.UninstallString) { throw 'Registro HKCU do Dita.se ausente ou incompleto.' }

    $exe = ($reg.DisplayIcon -replace ',-?\d+$').Trim('"')
    $local = [Environment]::GetFolderPath('LocalApplicationData').TrimEnd('\') + '\'
    if (-not $exe.StartsWith($local, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Instalou fora de LocalApplicationData: $exe"
    }
    if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) { throw "Executável ausente: $exe" }
    if ($VersaoEsperada -and $reg.DisplayVersion -ne $VersaoEsperada) {
        throw "Versão instalada: $($reg.DisplayVersion); esperada: $VersaoEsperada."
    }

    $atalho = Join-Path ([Environment]::GetFolderPath('Desktop')) 'Dita.se.lnk'
    if (-not (Test-Path -LiteralPath $atalho -PathType Leaf)) { throw "Atalho ausente: $atalho" }
    $shell = New-Object -ComObject WScript.Shell
    if ($shell.CreateShortcut($atalho).TargetPath -ine $exe) {
        throw "O atalho não aponta para $exe."
    }
    Write-Host "Instalado por usuário em $exe, com atalho válido."
    return $exe
}

$tokens = $null
$erros = $null
[void][System.Management.Automation.Language.Parser]::ParseInput($fonte, [ref]$tokens, [ref]$erros)
if ($erros.Count -gt 0) { throw ($erros -join [Environment]::NewLine) }
Write-Host "Sintaxe ok ($($PSVersionTable.PSEdition) $($PSVersionTable.PSVersion))."

$pasta = Join-Path ([IO.Path]::GetTempPath()) ("Dita.se instalação " + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $pasta | Out-Null
$envAnterior = @{
    DITASE_BASE_URL = $env:DITASE_BASE_URL
    DITASE_NAO_ABRIR = $env:DITASE_NAO_ABRIR
    TEMP = $env:TEMP
    TMP = $env:TMP
}
$env:DITASE_BASE_URL = $pasta
$env:DITASE_NAO_ABRIR = '1'
$env:TEMP = $pasta
$env:TMP = $pasta
$exeFixture = Join-Path $pasta 'Dita.se-windows-x64-setup.exe'
$somas = Join-Path $pasta 'SHA256SUMS.txt'

try {
    if ($Modo -eq 'sintaxe') {
        Set-Content -LiteralPath $exeFixture -Value 'arquivo de teste'
        Set-Content -LiteralPath $somas -Value ("0" * 64 + "  Dita.se-windows-x64-setup.exe")
        try {
            Invoke-Expression $fonte
        } catch {
            if ($_.Exception.Message -notlike '*Falha na conferência do SHA-256*') { throw }
            Write-Host "Recusou o hash incorreto: $($_.Exception.Message)"
            return
        }
        throw 'install.ps1 aceitou um hash incorreto.'
    }

    if (-not $Instalador -or -not (Test-Path -LiteralPath $Instalador -PathType Leaf)) {
        throw "Instalador não encontrado: $Instalador"
    }
    if (Registro-Teste) { throw 'Execute o teste em um usuário sem Dita.se instalado.' }

    Copy-Item -LiteralPath $Instalador -Destination $exeFixture
    $hash = (Get-FileHash -LiteralPath $exeFixture -Algorithm SHA256).Hash.ToLowerInvariant()
    Set-Content -LiteralPath $somas -Value "$hash  Dita.se-windows-x64-setup.exe" -Encoding Ascii

    # /D= fica por último e sem aspas por exigência do NSIS. A reinstalação pelo
    # script, sem /D=, precisa conservar essa pasta registrada.
    $destino = Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) ("Dita.se revisão " + [Guid]::NewGuid().ToString('N'))
    $instalacaoIniciada = $true
    $proc = Start-Process -FilePath $exeFixture -ArgumentList "/S /D=$destino" -Wait -PassThru
    if ($proc.ExitCode -ne 0) { throw "Primeira instalação falhou: código $($proc.ExitCode)." }
    $exe = Conferir-Instalacao
    if ((Split-Path -Parent $exe) -ine $destino) { throw "NSIS ignorou a pasta de teste: $exe" }

    Invoke-Expression $fonte
    if ((Conferir-Instalacao) -ine $exe) { throw 'A reinstalação mudou o caminho do aplicativo.' }
    Write-Host 'Reinstalação manteve a pasta com acento e espaço.'
} finally {
    try {
        if ($instalacaoIniciada) {
            $reg = Registro-Teste
            if ($reg) {
                if ($reg.UninstallString -notmatch '^"([^"]+)"\s*$') { throw 'Comando de desinstalação inesperado.' }
                $desinstalar = $Matches[1]
                if ((Split-Path -Parent $desinstalar) -ine $destino) { throw "Desinstalador fora da pasta de teste: $desinstalar" }
                $proc = Start-Process -FilePath $desinstalar -ArgumentList '/S' -Wait -PassThru
                if ($proc.ExitCode -ne 0) { throw "Desinstalação falhou: código $($proc.ExitCode)." }
                $atalho = Join-Path ([Environment]::GetFolderPath('Desktop')) 'Dita.se.lnk'
                if ((Registro-Teste) -or (Test-Path -LiteralPath $destino) -or (Test-Path -LiteralPath $atalho)) {
                    throw 'A desinstalação deixou registro, arquivos do app ou atalho.'
                }
                Write-Host 'Desinstalação removeu registro, arquivos do app e atalho.'
            }
        }
    } finally {
        foreach ($nome in $envAnterior.Keys) {
            [Environment]::SetEnvironmentVariable($nome, $envAnterior[$nome], 'Process')
        }
        Remove-Item -LiteralPath $pasta -Recurse -Force
    }
}
