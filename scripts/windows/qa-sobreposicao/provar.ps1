param(
    [Parameter(Mandatory)][string]$Instalador,
    [Parameter(Mandatory)][string]$Saida
)
$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force $Saida | Out-Null
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class JanelaQa {
    [StructLayout(LayoutKind.Sequential)]
    public struct Retangulo { public int X, Y, Dir, Base; }
    [StructLayout(LayoutKind.Sequential)]
    public struct Ponto { public int X, Y; public Ponto(int x, int y) { X = x; Y = y; } }
    private delegate bool EnumProc(IntPtr h, IntPtr arg);
    [DllImport("user32.dll")] private static extern bool EnumWindows(EnumProc f, IntPtr arg);
    [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr h, out uint p);
    [DllImport("user32.dll")] private static extern int GetWindowLong(IntPtr h, int indice);
    [DllImport("user32.dll")] private static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] private static extern IntPtr GetWindow(IntPtr h, uint codigo);
    [DllImport("user32.dll")] private static extern IntPtr WindowFromPoint(Ponto p);
    [DllImport("user32.dll")] private static extern IntPtr GetAncestor(IntPtr h, uint flags);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out Retangulo r);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
    [DllImport("user32.dll")] public static extern IntPtr SetProcessDpiAwarenessContext(IntPtr modo);
    public static IntPtr Ilha(uint pid) {
        IntPtr ilha = IntPtr.Zero;
        EnumWindows((h, arg) => {
            uint p;
            GetWindowThreadProcessId(h, out p);
            const int estilos = 0x08000088; // NOACTIVATE, TOOLWINDOW, TOPMOST.
            if (p != pid || !IsWindowVisible(h) || (GetWindowLong(h, -20) & estilos) != estilos)
                return true;
            Retangulo r;
            if (!GetWindowRect(h, out r) || r.Dir - r.X < 32 || r.Base - r.Y < 4)
                return true;
            ilha = h;
            return false;
        }, IntPtr.Zero);
        return ilha;
    }
    public static bool Acima(IntPtr ilha, IntPtr externa) {
        IntPtr atual = externa;
        for (int i = 0; i < 128; i++) {
            atual = GetWindow(atual, 3); // GW_HWNDPREV
            if (atual == IntPtr.Zero) return false;
            if (atual == ilha) return true;
        }
        return false;
    }
    public static IntPtr Alvo(int x, int y) { return GetAncestor(WindowFromPoint(new Ponto(x, y)), 2); }
}
'@
[void][JanelaQa]::SetProcessDpiAwarenessContext([IntPtr](-4))
$dir = Join-Path $Saida 'instalado'
$env:DITASE_RAIZ = Join-Path $Saida 'dados'
$rel = [ordered]@{ instaladorSHA256 = (Get-FileHash $Instalador).Hash; etapas = @() }
$app = $null
$form = $null
function Esperar([scriptblock]$acao, [int]$tempo = 5000) {
    $relogio = [Diagnostics.Stopwatch]::StartNew()
    do {
        [Windows.Forms.Application]::DoEvents()
        $resultado = & $acao
        if ($resultado) { return $resultado }
        Start-Sleep -Milliseconds 35
    } while ($relogio.ElapsedMilliseconds -lt $tempo)
}

function Foto($nome, $r) {
    $w = ($r.Dir - $r.X) + 28
    $h = ($r.Base - $r.Y) + 36
    $bitmap = New-Object Drawing.Bitmap($w, $h)
    $gfx = [Drawing.Graphics]::FromImage($bitmap)
    try {
        $gfx.CopyFromScreen(($r.X - 14), $r.Y, 0, 0, $bitmap.Size)
        $bitmap.Save((Join-Path $Saida "$nome.png"), [Drawing.Imaging.ImageFormat]::Png)
    } finally { $gfx.Dispose(); $bitmap.Dispose() }
}

function Medir($nome) {
    $ilha = [JanelaQa]::Ilha([uint32]$app.Id)
    $frente = [JanelaQa]::GetForegroundWindow()
    $resultado = [ordered]@{
        etapa = $nome; ilha = $ilha.ToInt64(); janelaEmFoco = $frente.ToInt64()
        janelaExterna = $externa.ToInt64(); acima = [JanelaQa]::Acima($ilha, $externa)
    }
    if ($ilha -ne [IntPtr]::Zero) {
        $r = New-Object JanelaQa+Retangulo
        [void][JanelaQa]::GetWindowRect($ilha, [ref]$r)
        $resultado['tamanho'] = @(($r.Dir - $r.X), ($r.Base - $r.Y))
        Foto $nome $r
    }
    $script:rel.etapas += $resultado
    Write-Host ($resultado | ConvertTo-Json -Compress)
    if ($ilha -eq [IntPtr]::Zero -or $frente -ne $externa -or -not $resultado.acima) {
        throw "Ilha invisível, atrás da janela ou foco indevido em $nome"
    }
    return $ilha
}

try {
    $inst = Start-Process -FilePath $Instalador -ArgumentList "/S /D=$dir" -Wait -PassThru
    if ($inst.ExitCode -ne 0) { throw "NSIS retornou $($inst.ExitCode)" }
    $exe = Get-ChildItem $dir -Filter '*.exe' | Where-Object { $_.Name -notmatch 'uninstall' } | Select-Object -First 1
    if (-not $exe) { throw 'Executável não encontrado' }
    $app = Start-Process $exe.FullName -PassThru
    $encontrou = Esperar {
        if ([JanelaQa]::Ilha([uint32]$app.Id) -ne [IntPtr]::Zero) { return $true }
    } 20000
    if (-not $encontrou) { throw 'Ilha ausente na abertura' }
    $tela = [Windows.Forms.Screen]::PrimaryScreen.Bounds
    [void][JanelaQa]::SetCursorPos(($tela.X + 100), ($tela.Y + 120))
    $form = New-Object Windows.Forms.Form
    $form.Text = 'QA Dita.se: janela sem borda'
    $form.BackColor = [Drawing.Color]::LightSkyBlue
    $form.StartPosition = 'Manual'
    $form.Bounds = New-Object Drawing.Rectangle(($tela.X + 80), ($tela.Y + 80), 360, 240)
    $form.Show()
    $externa = $form.Handle
    [JanelaQa]::keybd_event(0x12, 0, 0, [UIntPtr]::Zero)
    try { [void][JanelaQa]::SetForegroundWindow($externa) }
    finally { [JanelaQa]::keybd_event(0x12, 0, 2, [UIntPtr]::Zero) }
    $foco = Esperar { if ([JanelaQa]::GetForegroundWindow() -eq $externa) { return $true } }
    if (-not $foco) { throw 'A sessão de teste não deu foco à janela externa' }
    [void](Medir 'janela-com-moldura')
    Start-Sleep -Milliseconds 1200

    # Muda estilo e geometria com SetWindowLong/SetWindowPos sobre o HWND existente.
    $form.FormBorderStyle = 'None'
    $form.Bounds = $tela
    $form.TopMost = $true
    if ($form.Handle -ne $externa) { throw 'O cenário de teste trocou de HWND' }
    [void](Esperar {
        $h = [JanelaQa]::Ilha([uint32]$app.Id)
        if ($h -ne [IntPtr]::Zero -and [JanelaQa]::Acima($h, $externa)) { return $true }
    })
    $ilha = Medir 'mesmo-hwnd-sem-borda'
    $r = New-Object JanelaQa+Retangulo
    [void][JanelaQa]::GetWindowRect($ilha, [ref]$r)
    $meio = [int](($r.X + $r.Dir) / 2)
    [void][JanelaQa]::SetCursorPos($meio, ($r.Y + 7))
    $abriu = Esperar {
        [void][JanelaQa]::GetWindowRect($ilha, [ref]$r)
        if ($r.Base - $r.Y -gt 30) { return $true }
    }
    if (-not $abriu) { throw 'A ilha não abriu sob o ponteiro' }
    $ilha = Medir 'capsula-aberta'
    [void][JanelaQa]::GetWindowRect($ilha, [ref]$r)
    $canto = [JanelaQa]::Alvo($r.X, ($r.Base - 1))
    $centro = [JanelaQa]::Alvo($meio, [int](($r.Y + $r.Base) / 2))
    $rel.alvos = @{ canto = $canto.ToInt64(); centro = $centro.ToInt64() }
    if ($canto -ne $externa -or $centro -ne $ilha) { throw 'O teste de clique encontrou uma janela inesperada' }
} finally {
    if ($form) { $form.Dispose() }
    if ($app -and -not $app.HasExited) { $app.Kill() }
    $rel | ConvertTo-Json -Depth 5 | Set-Content (Join-Path $Saida 'sobreposicao.json') -Encoding UTF8
}
