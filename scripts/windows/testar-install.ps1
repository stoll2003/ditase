# Testes do scripts/install.ps1, rodados no CI do Windows.
#   -Modo sintaxe     : confere a sintaxe e roda pelo caminho do irm | iex contra uma
#                       pasta local com hash errado (tem de recusar sem instalar).
#   -Modo instalacao  : instala de verdade o instalador gerado no CI (-Instalador),
#                       confere registro, executável e atalho, e desinstala.
param(
    [ValidateSet('sintaxe', 'instalacao')]
    [string]$Modo = 'sintaxe',
    [string]$Instalador
)

$ErrorActionPreference = 'Stop'
$script = Join-Path $PSScriptRoot '..\install.ps1' | Resolve-Path
$fonte = Get-Content -LiteralPath $script -Raw -Encoding UTF8
$env:DITASE_NAO_ABRIR = '1'

$tokens = $null; $erros = $null
[void][System.Management.Automation.Language.Parser]::ParseInput($fonte, [ref]$tokens, [ref]$erros)
if ($erros.Count -gt 0) {
    $erros | ForEach-Object { Write-Host "::error::install.ps1: $($_.Message) (linha $($_.Extent.StartLineNumber))" }
    exit 1
}
Write-Host "Sintaxe ok ($($PSVersionTable.PSEdition) $($PSVersionTable.PSVersion))."

$pasta = Join-Path ([IO.Path]::GetTempPath()) ("ditase-teste-" + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $pasta | Out-Null
$env:DITASE_BASE_URL = $pasta

if ($Modo -eq 'sintaxe') {
    Set-Content -LiteralPath (Join-Path $pasta 'Dita.se-windows-x64-setup.exe') -Value 'nao e um instalador'
    Set-Content -LiteralPath (Join-Path $pasta 'SHA256SUMS.txt') -Value ("0" * 64 + "  Dita.se-windows-x64-setup.exe")
    $recusou = $false
    try {
        Invoke-Expression $fonte
    } catch {
        if ($_.Exception.Message -like '*SHA-256*') { $recusou = $true; Write-Host "Recusou como esperado: $($_.Exception.Message)" }
        else { throw }
    }
    if (-not $recusou) { Write-Host '::error::install.ps1 aceitou um hash errado.'; exit 1 }
    exit 0
}

# Modo instalacao
if (-not $Instalador -or -not (Test-Path -LiteralPath $Instalador)) { throw "Instalador nao encontrado: $Instalador" }
Copy-Item -LiteralPath $Instalador -Destination (Join-Path $pasta 'Dita.se-windows-x64-setup.exe')
$hash = (Get-FileHash -LiteralPath (Join-Path $pasta 'Dita.se-windows-x64-setup.exe') -Algorithm SHA256).Hash.ToLowerInvariant()
Set-Content -LiteralPath (Join-Path $pasta 'SHA256SUMS.txt') -Value "$hash  Dita.se-windows-x64-setup.exe"

Invoke-Expression $fonte

$reg = Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*' -ErrorAction SilentlyContinue |
    Where-Object { $_.PSObject.Properties['DisplayName'] -and $_.DisplayName -like 'Dita.se*' } | Select-Object -First 1
if (-not $reg) { throw 'Registro HKCU do Dita.se ausente apos instalar.' }
$exe = ($reg.DisplayIcon -split ',')[0].Trim('"')
if (-not $exe.StartsWith($env:LOCALAPPDATA, [StringComparison]::OrdinalIgnoreCase)) {
    throw "Instalou fora de %LOCALAPPDATA% (por usuario): $exe"
}
$atalho = Join-Path ([Environment]::GetFolderPath('Desktop')) "$($reg.DisplayName).lnk"
if (-not (Test-Path -LiteralPath $atalho)) { throw "Atalho ausente: $atalho" }
Write-Host "Instalado por usuario em $exe, atalho $atalho."

if ($reg.UninstallString) {
    $desinstalar = ($reg.UninstallString -replace '^"([^"]+)".*$', '$1')
    Start-Process -FilePath $desinstalar -ArgumentList '/S' -Wait
}
