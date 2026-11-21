param(
    [Parameter(Mandatory = $true)]
    [string]$Arquivo,
    [string]$Titulo = 'Escolher áudio ou vídeo',
    [int]$TimeoutSegundos = 20,
    [string]$ArvoreSaida = ''
)

# Seleciona um arquivo no diálogo nativo de abrir do Windows pela UI Automation.
# Run 44875618743 (Windows Server 2025): a busca por nome em Descendants achou um elemento que não
# era a janela, e o campo 1148 não estava sob ele. Agora procura a janela #32770 entre as janelas
# de topo, aceita o campo 1148 como ComboBox ou Edit e, sem ValuePattern, digita pelo teclado.
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -AssemblyName System.Windows.Forms
$AE = [System.Windows.Automation.AutomationElement]
$TS = [System.Windows.Automation.TreeScope]
$CT = [System.Windows.Automation.ControlType]

function Cond($prop, $valor) { New-Object System.Windows.Automation.PropertyCondition($prop, $valor) }

function Gravar-Arvore($raiz) {
    if (-not $ArvoreSaida -or -not $raiz) { return }
    $todos = $raiz.FindAll($TS::Descendants, [System.Windows.Automation.Condition]::TrueCondition)
    $linhas = foreach ($e in $todos) {
        $c = $e.Current
        "{0} | id={1} | nome={2} | classe={3}" -f $c.ControlType.ProgrammaticName, $c.AutomationId, $c.Name, $c.ClassName
    }
    $linhas | Select-Object -First 300 | Set-Content -LiteralPath $ArvoreSaida -Encoding UTF8
}

$caminho = (Resolve-Path -LiteralPath $Arquivo).Path
$fim = (Get-Date).AddSeconds($TimeoutSegundos)
$janela = $null
while ((Get-Date) -lt $fim -and -not $janela) {
    # Diálogo comum: janela #32770 de topo (às vezes filha da janela do app).
    $dialogos = $AE::RootElement.FindAll($TS::Descendants, (Cond $AE::ClassNameProperty '#32770'))
    foreach ($d in $dialogos) {
        if ($d.Current.Name -eq $Titulo -or -not $janela) { $janela = $d }
        if ($d.Current.Name -eq $Titulo) { break }
    }
    if (-not $janela) { Start-Sleep -Milliseconds 250 }
}
if (-not $janela) { throw "Diálogo nativo (#32770) não apareceu em ${TimeoutSegundos}s." }
Write-Host "Diálogo: nome='$($janela.Current.Name)' classe=$($janela.Current.ClassName)"

$campo = $null
$fimCampo = (Get-Date).AddSeconds(10)
while ((Get-Date) -lt $fimCampo -and -not $campo) {
    $cands = $janela.FindAll($TS::Descendants, (Cond $AE::AutomationIdProperty '1148'))
    foreach ($c in $cands) {
        if ($c.Current.ControlType -eq $CT::Edit) { $campo = $c; break }
    }
    if (-not $campo) {
        foreach ($c in $cands) {
            if ($c.Current.ControlType -eq $CT::ComboBox) {
                $edit = $c.FindFirst($TS::Descendants, (Cond $AE::ControlTypeProperty $CT::Edit))
                $campo = if ($edit) { $edit } else { $c }
                break
            }
        }
    }
    if (-not $campo) { Start-Sleep -Milliseconds 250 }
}
if (-not $campo) {
    Gravar-Arvore $janela
    throw "Campo de nome de arquivo (AutomationId 1148) não encontrado no diálogo '$($janela.Current.Name)'."
}

$vp = $null
if ($campo.TryGetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern, [ref]$vp)) {
    $vp.SetValue($caminho)
    Start-Sleep -Milliseconds 200
    $abrir = $janela.FindFirst($TS::Descendants, (New-Object System.Windows.Automation.AndCondition(
        (Cond $AE::AutomationIdProperty '1'), (Cond $AE::ControlTypeProperty $CT::Button))))
    $ip = $null
    if ($abrir -and $abrir.TryGetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern, [ref]$ip)) {
        $ip.Invoke()
    } else {
        $campo.SetFocus(); [System.Windows.Forms.SendKeys]::SendWait('{ENTER}')
    }
} else {
    # Sem ValuePattern: foco no campo e digitação. Caracteres especiais do SendKeys escapados.
    $campo.SetFocus()
    $texto = $caminho -replace '([+^%~(){}\[\]])', '{$1}'
    [System.Windows.Forms.SendKeys]::SendWait($texto)
    Start-Sleep -Milliseconds 200
    [System.Windows.Forms.SendKeys]::SendWait('{ENTER}')
}

# Confirma que o diálogo fechou (o app recebeu o arquivo).
$fimFechar = (Get-Date).AddSeconds(10)
do {
    $aberto = $false
    try { $null = $janela.Current.Name; $aberto = -not $janela.Current.IsOffscreen } catch { $aberto = $false }
    if (-not $aberto) { break }
    Start-Sleep -Milliseconds 250
} while ((Get-Date) -lt $fimFechar)
if ($aberto) { Gravar-Arvore $janela; throw 'O diálogo continuou aberto depois de enviar o caminho.' }
Write-Host "Arquivo selecionado no diálogo nativo: $caminho"
