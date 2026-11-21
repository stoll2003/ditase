param(
    [Parameter(Mandatory = $true, Position = 0)]
    [ValidateSet('notepad', 'mover', 'clicar', 'texto', 'filmar', 'entrada')]
    [string]$Acao,
    [int]$X = 0, [int]$Y = 0, [int]$W = 0, [int]$H = 0,
    [int64]$Janela = 0,
    [string]$Pasta = '',
    [string]$Parar = '',
    [int]$Fps = 12,
    [int]$MaxSegundos = 180
)

# Auxiliar de desktop da prova do ditado (pixels físicos, per-monitor v2):
#   notepad            abre o Bloco de Notas maximizado em primeiro plano e imprime o hwnd
#   mover / clicar     ponteiro real (SetCursorPos + mouse_event) em X,Y
#   texto              imprime o texto do Bloco de Notas -Janela (Edit ou RichEdit)
#   filmar             grava a região X,Y,W,H em PNG (-Pasta) a -Fps até existir o arquivo -Parar
#   entrada            lista os dispositivos de captura (o VB-CABLE "CABLE Output" vira o microfone)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class QaDesk {
    public delegate bool EnumProc(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] public static extern IntPtr SetProcessDpiAwarenessContext(IntPtr v);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, uint d, UIntPtr e);
    [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint f, UIntPtr e);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int c);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool EnumChildWindows(IntPtr p, EnumProc f, IntPtr l);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc f, IntPtr l);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetClassName(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern IntPtr SendMessage(IntPtr h, uint m, IntPtr w, StringBuilder l);
    [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
    public static string Classe(IntPtr h) { var s = new StringBuilder(256); GetClassName(h, s, 256); return s.ToString(); }
    public static IntPtr Notepad(uint pid) {
        IntPtr achado = IntPtr.Zero;
        EnumWindows((h, _) => { uint p; GetWindowThreadProcessId(h, out p);
            if (IsWindowVisible(h) && Classe(h) == "Notepad" && (pid == 0 || p == pid)) { achado = h; return false; } return true; }, IntPtr.Zero);
        return achado;
    }
    public static string Texto(IntPtr janela) {
        var sb = new StringBuilder();
        EnumChildWindows(janela, (h, _) => {
            string c = Classe(h);
            if (c == "Edit" || c.StartsWith("RichEdit")) {
                int n = (int)SendMessage(h, 0x000E, IntPtr.Zero, IntPtr.Zero);  // WM_GETTEXTLENGTH
                var t = new StringBuilder(n + 2);
                SendMessage(h, 0x000D, (IntPtr)(n + 1), t);                       // WM_GETTEXT
                sb.Append(t.ToString());
            }
            return true;
        }, IntPtr.Zero);
        return sb.ToString();
    }
}
'@
[void][QaDesk]::SetProcessDpiAwarenessContext([IntPtr](-4))

switch ($Acao) {
    'notepad' {
        $p = Start-Process notepad.exe -PassThru
        $h = [IntPtr]::Zero; $fim = (Get-Date).AddSeconds(15)
        while ($h -eq [IntPtr]::Zero -and (Get-Date) -lt $fim) { Start-Sleep -Milliseconds 300; $h = [QaDesk]::Notepad(0) }
        if ($h -eq [IntPtr]::Zero) { throw 'Bloco de Notas não apareceu' }
        [void][QaDesk]::ShowWindow($h, 3)
        [QaDesk]::keybd_event(0x12, 0, 0, [UIntPtr]::Zero); [QaDesk]::keybd_event(0x12, 0, 2, [UIntPtr]::Zero)
        [void][QaDesk]::SetForegroundWindow($h)
        Start-Sleep -Milliseconds 400
        [ordered]@{ hwnd = [int64]$h; primeiroPlano = ([QaDesk]::GetForegroundWindow() -eq $h) } | ConvertTo-Json -Compress
    }
    'mover' {
        [void][QaDesk]::SetCursorPos($X - 2, $Y); Start-Sleep -Milliseconds 40
        [QaDesk]::mouse_event(0x0001, 2, 0, 0, [UIntPtr]::Zero)
        'ok'
    }
    'clicar' {
        [void][QaDesk]::SetCursorPos($X, $Y); Start-Sleep -Milliseconds 80
        [QaDesk]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero); Start-Sleep -Milliseconds 40
        [QaDesk]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)
        [ordered]@{ primeiroPlano = [QaDesk]::Classe([QaDesk]::GetForegroundWindow()) } | ConvertTo-Json -Compress
    }
    'texto' {
        $t = [QaDesk]::Texto([IntPtr]$Janela)
        [ordered]@{ texto = $t; primeiroPlano = [QaDesk]::Classe([QaDesk]::GetForegroundWindow()) } | ConvertTo-Json -Compress
    }
    'entrada' {
        @(Get-PnpDevice -Class AudioEndpoint -Status OK -ErrorAction SilentlyContinue | ForEach-Object { $_.FriendlyName }) | ConvertTo-Json -Compress
    }
    'filmar' {
        New-Item -ItemType Directory -Force -Path $Pasta | Out-Null
        $bmp = New-Object System.Drawing.Bitmap $W, $H
        $g = [System.Drawing.Graphics]::FromImage($bmp)
        $intervalo = [int](1000 / [Math]::Max(1, $Fps))
        $t0 = [DateTime]::UtcNow; $n = 0; $tempos = New-Object System.Collections.Generic.List[double]
        while (-not (Test-Path -LiteralPath $Parar) -and ([DateTime]::UtcNow - $t0).TotalSeconds -lt $MaxSegundos) {
            $ini = [DateTime]::UtcNow
            $g.CopyFromScreen($X, $Y, 0, 0, $bmp.Size)
            $bmp.Save((Join-Path $Pasta ('{0:D5}.png' -f $n)), [System.Drawing.Imaging.ImageFormat]::Png)
            $tempos.Add([Math]::Round(([DateTimeOffset]$ini).ToUnixTimeMilliseconds() / 1000.0, 3)); $n++
            $resto = $intervalo - [int]([DateTime]::UtcNow - $ini).TotalMilliseconds
            if ($resto -gt 0) { Start-Sleep -Milliseconds $resto }
        }
        $g.Dispose(); $bmp.Dispose()
        [ordered]@{ quadros = $n; fps = $Fps; regiao = @($X, $Y, $W, $H); tempos = $tempos } | ConvertTo-Json -Compress |
            Set-Content -LiteralPath (Join-Path $Pasta 'quadros.json') -Encoding UTF8
    }
}
