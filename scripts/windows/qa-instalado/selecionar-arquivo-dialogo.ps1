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
    # Run 44876775586: no runner os controles vêm como Pane (proxy MSAA); vale a classe Win32.
    foreach ($c in $cands) {
        if ($c.Current.ControlType -eq $CT::Edit -or $c.Current.ClassName -eq 'Edit') { $campo = $c; break }
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

Add-Type -Namespace QaDialogo -Name Win32 -MemberDefinition @'
[System.Runtime.InteropServices.DllImport("user32.dll", CharSet = System.Runtime.InteropServices.CharSet.Unicode)]
public static extern System.IntPtr SendMessage(System.IntPtr h, uint msg, System.IntPtr w, string l);
[System.Runtime.InteropServices.DllImport("user32.dll")]
public static extern System.IntPtr SendMessage(System.IntPtr h, uint msg, System.IntPtr w, System.IntPtr l);
[System.Runtime.InteropServices.DllImport("user32.dll")]
public static extern bool IsWindow(System.IntPtr h);
[System.Runtime.InteropServices.DllImport("user32.dll")]
public static extern bool IsWindowVisible(System.IntPtr h);
'@
$hDialogo = [IntPtr]$janela.Current.NativeWindowHandle
$abrir = $janela.FindFirst($TS::Descendants, (New-Object System.Windows.Automation.AndCondition(
    (Cond $AE::AutomationIdProperty '1'), (Cond $AE::ClassNameProperty 'Button'))))
$hCampo = [IntPtr]$campo.Current.NativeWindowHandle
$hAbrir = if ($abrir) { [IntPtr]$abrir.Current.NativeWindowHandle } else { [IntPtr]::Zero }
Write-Host "Campo: tipo=$($campo.Current.ControlType.ProgrammaticName) classe=$($campo.Current.ClassName) hwnd=$hCampo; Abrir hwnd=$hAbrir"

$vp = $null
if ($hCampo -ne [IntPtr]::Zero -and $hAbrir -ne [IntPtr]::Zero) {
    # Mensagens nativas do próprio controle do diálogo: WM_SETTEXT no campo e BM_CLICK em Abrir.
    [void][QaDialogo.Win32]::SendMessage($hCampo, 0x000C, [IntPtr]::Zero, $caminho)
    Start-Sleep -Milliseconds 200
    # Run 44877843973: BM_CLICK não fechou (diálogo sem foco no runner; a doc avisa que BM_CLICK
    # pode falhar em diálogo inativo). WM_COMMAND(IDOK, BN_CLICKED) vai direto ao procedimento do diálogo.
    [void][QaDialogo.Win32]::SendMessage($hDialogo, 0x0111, [IntPtr]1, $hAbrir)
} elseif ($campo.TryGetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern, [ref]$vp)) {
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
    $aberto = [QaDialogo.Win32]::IsWindow($hDialogo) -and [QaDialogo.Win32]::IsWindowVisible($hDialogo)
    if (-not $aberto) { break }
    Start-Sleep -Milliseconds 250
} while ((Get-Date) -lt $fimFechar)
if ($aberto) {
    $buf = New-Object System.Text.StringBuilder 1024
    Gravar-Arvore $janela
    throw "O diálogo continuou aberto depois de enviar o caminho (hwnd=$hDialogo, campo=$hCampo)."
}
Write-Host "Arquivo selecionado no diálogo nativo: $caminho"
