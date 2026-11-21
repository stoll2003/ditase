param(
    [Parameter(Mandatory = $true)]
    [string]$Arquivo,
    [string]$Titulo = 'Escolher áudio ou vídeo',
    [int]$TimeoutSegundos = 20
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes

$caminho = (Resolve-Path -LiteralPath $Arquivo).Path
$fim = (Get-Date).AddSeconds($TimeoutSegundos)
$janela = $null

while ((Get-Date) -lt $fim -and -not $janela) {
    $cond = New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::NameProperty,
        $Titulo
    )
    $janela = [System.Windows.Automation.AutomationElement]::RootElement.FindFirst(
        [System.Windows.Automation.TreeScope]::Descendants,
        $cond
    )
    if (-not $janela) { Start-Sleep -Milliseconds 250 }
}

if (-not $janela) {
    throw "Diálogo nativo '$Titulo' não apareceu em ${TimeoutSegundos}s."
}

# Campo "Nome:" do diálogo padrão de arquivo do Windows (AutomationId 1148).
$condCampo = New-Object System.Windows.Automation.AndCondition(
    (New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::AutomationIdProperty, '1148'
    )),
    (New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
        [System.Windows.Automation.ControlType]::Edit
    ))
)
$campo = $janela.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $condCampo)
if (-not $campo) {
    throw 'Campo de nome de arquivo (AutomationId 1148) não encontrado no diálogo nativo.'
}

$vp = $campo.GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern)
$vp.SetValue($caminho)
Start-Sleep -Milliseconds 200

# Botão "Abrir" / "Open" (AutomationId 1).
$condAbrir = New-Object System.Windows.Automation.AndCondition(
    (New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::AutomationIdProperty, '1'
    )),
    (New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
        [System.Windows.Automation.ControlType]::Button
    ))
)
$btnAbrir = $janela.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $condAbrir)
if (-not $btnAbrir) {
    throw 'Botão Abrir (AutomationId 1) não encontrado no diálogo nativo.'
}

$inv = $btnAbrir.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern)
$inv.Invoke()
Write-Host "Arquivo selecionado no diálogo nativo: $caminho"
