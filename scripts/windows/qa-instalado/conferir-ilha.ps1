param(
    [Parameter(Mandatory = $true)][int]$ProcessoApp,
    [Parameter(Mandatory = $true)][string]$Saida,
    [string]$Executavel = '',
    [int[]]$Escalas = @(100, 125, 150, 175, 200, 250)
)

# Conferência nativa da ilha instalada (spec SPEC-dynamic-island.md §6), com o app aberto:
# - posição e tamanho físicos: centro da área de trabalho do monitor principal, y = work_y,
#   w/h = ceil(lógico * escala) (repouso e aceno 72 x 14; aberta 188 x 44);
# - recorte: região da janela (GetWindowRgn/PtInRegion) e WindowFromPoint nos cantos;
# - clique através dos cantos e foco: Bloco de Notas maximizado em primeiro plano continua em
#   primeiro plano depois de hover e clique na ilha, e o clique no canto cai nele;
# - escalas: troca a escala do monitor em tempo de execução (DisplayConfigSetDeviceInfo, a mesma
#   chamada do painel de Configurações) para cada escala que o monitor do runner aceitar, mede e
#   restaura a original no final. Grava ilha.json e recortes PNG; código 1 se algo obrigatório falhar.
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

public static class QaIlha {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
    [StructLayout(LayoutKind.Sequential)] public struct MONITORINFO { public int cbSize; public RECT rcMonitor; public RECT rcWork; public uint dwFlags; }
    public delegate bool EnumProc(IntPtr h, IntPtr l);

    [DllImport("user32.dll")] public static extern IntPtr SetProcessDpiAwarenessContext(IntPtr v);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc f, IntPtr l);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern int GetWindowLong(IntPtr h, int i);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern IntPtr MonitorFromPoint(POINT p, uint f);
    [DllImport("user32.dll")] public static extern bool GetMonitorInfo(IntPtr m, ref MONITORINFO i);
    [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(POINT p);
    [DllImport("user32.dll")] public static extern IntPtr GetAncestor(IntPtr h, uint f);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int c);
    [DllImport("user32.dll")] public static extern int GetWindowRgn(IntPtr h, IntPtr r);
    [DllImport("gdi32.dll")] public static extern IntPtr CreateRectRgn(int a, int b, int c, int d);
    [DllImport("gdi32.dll")] public static extern bool PtInRegion(IntPtr r, int x, int y);
    [DllImport("gdi32.dll")] public static extern bool DeleteObject(IntPtr o);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, uint d, UIntPtr e);
    [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint f, UIntPtr e);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetClassName(IntPtr h, StringBuilder s, int n);

    public static List<IntPtr> JanelasDo(uint pid) {
        var l = new List<IntPtr>();
        EnumWindows((h, _) => { uint p; GetWindowThreadProcessId(h, out p); if (p == pid) l.Add(h); return true; }, IntPtr.Zero);
        return l;
    }
    public static string Classe(IntPtr h) { var s = new StringBuilder(256); GetClassName(h, s, 256); return s.ToString(); }
    public static int Regiao(IntPtr h, int x, int y, out bool dentro) {
        IntPtr r = CreateRectRgn(0, 0, 0, 0);
        int t = GetWindowRgn(h, r);
        dentro = t > 1 && PtInRegion(r, x, y);
        DeleteObject(r);
        return t;
    }
    public static void Clicar(int x, int y) {
        SetCursorPos(x, y); System.Threading.Thread.Sleep(80);
        mouse_event(0x0002, 0, 0, 0, UIntPtr.Zero); System.Threading.Thread.Sleep(40);
        mouse_event(0x0004, 0, 0, 0, UIntPtr.Zero);
    }

    // Escala do monitor (API de configuração de vídeo; mesma usada pelas Configurações do Windows).
    [StructLayout(LayoutKind.Sequential)] public struct LUID { public uint Low; public int High; }
    [StructLayout(LayoutKind.Sequential)] public struct RATIONAL { public uint N, D; }
    [StructLayout(LayoutKind.Sequential)] public struct PATH_SOURCE { public LUID adapterId; public uint id; public uint modeIdx; public uint status; }
    [StructLayout(LayoutKind.Sequential)] public struct PATH_TARGET { public LUID adapterId; public uint id; public uint modeIdx; public uint tech; public uint rot; public uint scaling; public RATIONAL refresh; public uint scan; public int avail; public uint status; }
    [StructLayout(LayoutKind.Sequential)] public struct PATH_INFO { public PATH_SOURCE src; public PATH_TARGET tgt; public uint flags; }
    [StructLayout(LayoutKind.Explicit, Size = 64)] public struct MODE_INFO { [FieldOffset(0)] public uint type; }
    [StructLayout(LayoutKind.Sequential)] public struct HEADER { public int type; public uint size; public LUID adapterId; public uint id; }
    [StructLayout(LayoutKind.Sequential)] public struct DPI_GET { public HEADER h; public int min; public int cur; public int max; }
    [StructLayout(LayoutKind.Sequential)] public struct DPI_SET { public HEADER h; public int rel; }
    [DllImport("user32.dll")] static extern int GetDisplayConfigBufferSizes(uint f, out uint np, out uint nm);
    [DllImport("user32.dll")] static extern int QueryDisplayConfig(uint f, ref uint np, [Out] PATH_INFO[] p, ref uint nm, [Out] MODE_INFO[] m, IntPtr t);
    [DllImport("user32.dll")] static extern int DisplayConfigGetDeviceInfo(ref DPI_GET g);
    [DllImport("user32.dll")] static extern int DisplayConfigSetDeviceInfo(ref DPI_SET s);
    [DllImport("user32.dll")] public static extern IntPtr GetWindowDpiAwarenessContext(IntPtr h);
    [DllImport("user32.dll")] public static extern int GetAwarenessFromDpiAwarenessContext(IntPtr c);
    // 0 = sem DPI, 1 = sistema, 2 = por monitor (v1 ou v2).
    public static int ModoDpi(IntPtr h) { return GetAwarenessFromDpiAwarenessContext(GetWindowDpiAwarenessContext(h)); }
    [DllImport("shcore.dll")] public static extern int GetDpiForMonitor(IntPtr m, int t, out uint x, out uint y);
    [DllImport("user32.dll")] public static extern IntPtr MonitorFromWindow(IntPtr h, uint f);
    public static uint DpiDoMonitor(IntPtr h) { uint x, y; GetDpiForMonitor(MonitorFromWindow(h, 2), 0, out x, out y); return x; }

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Ansi)]
    public struct DEVMODE {
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 32)] public string dmDeviceName;
        public short dmSpecVersion, dmDriverVersion, dmSize, dmDriverExtra; public int dmFields;
        public int dmPositionX, dmPositionY, dmDisplayOrientation, dmDisplayFixedOutput;
        public short dmColor, dmDuplex, dmYResolution, dmTTOption, dmCollate;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 32)] public string dmFormName;
        public short dmLogPixels; public int dmBitsPerPel, dmPelsWidth, dmPelsHeight, dmDisplayFlags, dmDisplayFrequency;
        public int dmICMMethod, dmICMIntent, dmMediaType, dmDitherType, dmReserved1, dmReserved2, dmPanningWidth, dmPanningHeight;
    }
    [DllImport("user32.dll", CharSet = CharSet.Ansi)] static extern bool EnumDisplaySettings(string d, int i, ref DEVMODE m);
    [DllImport("user32.dll", CharSet = CharSet.Ansi)] static extern int ChangeDisplaySettingsEx(string d, ref DEVMODE m, IntPtr h, uint f, IntPtr l);
    [DllImport("user32.dll", CharSet = CharSet.Ansi)] static extern int ChangeDisplaySettingsEx(string d, IntPtr m, IntPtr h, uint f, IntPtr l);
    // Maior modo listado pelo adaptador; devolve "LxA código" ou "" se não houver maior.
    public static string AumentarResolucao() {
        var atual = new DEVMODE(); atual.dmSize = (short)Marshal.SizeOf(typeof(DEVMODE));
        EnumDisplaySettings(null, -1, ref atual);
        DEVMODE melhor = atual; bool achou = false;
        for (int i = 0; ; i++) {
            var m = new DEVMODE(); m.dmSize = (short)Marshal.SizeOf(typeof(DEVMODE));
            if (!EnumDisplaySettings(null, i, ref m)) break;
            if ((long)m.dmPelsWidth * m.dmPelsHeight > (long)melhor.dmPelsWidth * melhor.dmPelsHeight && m.dmPelsHeight <= 2160) { melhor = m; achou = true; }
        }
        if (!achou) return "";
        melhor.dmFields = 0x80000 | 0x100000;  // DM_PELSWIDTH | DM_PELSHEIGHT
        int r = ChangeDisplaySettingsEx(null, ref melhor, IntPtr.Zero, 0, IntPtr.Zero);
        return melhor.dmPelsWidth + "x" + melhor.dmPelsHeight + " " + r;
    }
    public static int RestaurarResolucao() { return ChangeDisplaySettingsEx(null, IntPtr.Zero, IntPtr.Zero, 0, IntPtr.Zero); }
    public static readonly int[] Valores = { 100, 125, 150, 175, 200, 225, 250, 300, 350, 400, 450, 500 };

    static PATH_SOURCE Fonte() {
        uint np, nm;
        if (GetDisplayConfigBufferSizes(2, out np, out nm) != 0) throw new Exception("GetDisplayConfigBufferSizes");
        var p = new PATH_INFO[np]; var m = new MODE_INFO[nm];
        if (QueryDisplayConfig(2, ref np, p, ref nm, m, IntPtr.Zero) != 0) throw new Exception("QueryDisplayConfig");
        if (np == 0) throw new Exception("sem caminho de vídeo ativo");
        return p[0].src;
    }
    // Devolve {recomendada, atual, mínima, máxima} em %.
    public static int[] LerEscala() {
        var s = Fonte();
        var g = new DPI_GET(); g.h.type = -3; g.h.size = (uint)Marshal.SizeOf(typeof(DPI_GET)); g.h.adapterId = s.adapterId; g.h.id = s.id;
        int r = DisplayConfigGetDeviceInfo(ref g);
        if (r != 0) throw new Exception("DisplayConfigGetDeviceInfo " + r);
        int rec = Math.Abs(g.min);
        return new[] { Valores[rec], Valores[rec + g.cur], Valores[0], Valores[Math.Min(Valores.Length - 1, rec + g.max)] };
    }
    public static int DefinirEscala(int pct) {
        var s = Fonte();
        var g = new DPI_GET(); g.h.type = -3; g.h.size = (uint)Marshal.SizeOf(typeof(DPI_GET)); g.h.adapterId = s.adapterId; g.h.id = s.id;
        DisplayConfigGetDeviceInfo(ref g);
        int rec = Math.Abs(g.min), alvo = Array.IndexOf(Valores, pct);
        if (alvo < 0) return -1;
        var st = new DPI_SET(); st.h.type = -4; st.h.size = (uint)Marshal.SizeOf(typeof(DPI_SET)); st.h.adapterId = s.adapterId; st.h.id = s.id; st.rel = alvo - rec;
        return DisplayConfigSetDeviceInfo(ref st);
    }
}
'@

[void][QaIlha]::SetProcessDpiAwarenessContext([IntPtr](-4))  # Per-monitor v2: tudo em pixels físicos.
New-Item -ItemType Directory -Force -Path $Saida | Out-Null
$rel = [ordered]@{ checagens = @(); escalas = @(); limites = @() }
$falhas = 0
function Checar($nome, $ok, $detalhe, [switch]$Opcional) {
    $script:rel.checagens += [ordered]@{ nome = $nome; ok = [bool]$ok; obrigatoria = -not $Opcional; detalhe = $detalhe }
    if (-not $ok -and -not $Opcional) { $script:falhas++ }
    Write-Host ("[{0}] {1} {2}" -f ($(if ($ok) { 'ok' } else { 'FALHA' })), $nome, ($detalhe | ConvertTo-Json -Compress -Depth 4))
}

function Listar-Candidatas {
    foreach ($h in [QaIlha]::JanelasDo([uint32]$ProcessoApp)) {
        $ex = [QaIlha]::GetWindowLong($h, -20)
        if (($ex -band 0x08000000) -eq 0) { continue }
        $r = New-Object QaIlha+RECT; [void][QaIlha]::GetWindowRect($h, [ref]$r)
        $dummy = $false; $tipo = [QaIlha]::Regiao($h, 0, 0, [ref]$dummy)
        [pscustomobject]@{ hwnd = $h; classe = [QaIlha]::Classe($h); exStyle = ('0x{0:X8}' -f $ex); visivel = [QaIlha]::IsWindowVisible($h)
            regiao = $tipo; x = $r.L; y = $r.T; w = $r.R - $r.L; h = $r.B - $r.T }
    }
}

# A ilha é a janela visível do processo com WS_EX_NOACTIVATE E região de recorte (só ela recebe
# SetWindowRgn). Run 44885109477: há outra janela NOACTIVATE de 18-24 px em (0,0), e o patch de
# escala recria a ilha (hwnd novo); por isso a prova procura de novo a cada medição.
function Achar-Ilha {
    $c = @(Listar-Candidatas)
    $comRegiao = @($c | Where-Object { $_.visivel -and $_.regiao -ge 2 } | Sort-Object { $_.w * $_.h } -Descending)
    if ($comRegiao.Count) { return $comRegiao[0].hwnd }
    return [IntPtr]::Zero
}

function Medir($h) {
    $r = New-Object QaIlha+RECT
    [void][QaIlha]::GetWindowRect($h, [ref]$r)
    $p = New-Object QaIlha+POINT; $p.X = 0; $p.Y = 0
    $mon = [QaIlha]::MonitorFromPoint($p, 1)  # MONITOR_DEFAULTTOPRIMARY
    $mi = New-Object QaIlha+MONITORINFO; $mi.cbSize = [Runtime.InteropServices.Marshal]::SizeOf($mi)
    [void][QaIlha]::GetMonitorInfo($mon, [ref]$mi)
    [ordered]@{
        t = [Math]::Round([DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds() / 1000.0, 3)  # mesmo relógio do ditado.log
        visivel = [QaIlha]::IsWindowVisible($h)
        modoDpi = [QaIlha]::ModoDpi($h)
        regiaoTipo = $(& { $x = $false; [QaIlha]::Regiao($h, 0, 0, [ref]$x) })
        x = $r.L; y = $r.T; w = $r.R - $r.L; h = $r.B - $r.T
        escala = [QaIlha]::GetDpiForWindow($h) / 96.0
        workX = $mi.rcWork.L; workY = $mi.rcWork.T; workW = $mi.rcWork.R - $mi.rcWork.L
    }
}

function Esperado($m, $wLog, $hLog) {
    $w = [int][Math]::Ceiling($wLog * $m.escala)
    $h = [int][Math]::Ceiling($hLog * $m.escala)
    $x = $m.workX + [int][Math]::Round(($m.workW - $w) / 2.0, [MidpointRounding]::AwayFromZero)
    [ordered]@{ x = $x; y = $m.workY; w = $w; h = $h }
}

function Bate($m, $e) {
    # Tolerância de 1 px no x pelo arredondamento de .5 (Rust round = away from zero).
    ([Math]::Abs($m.x - $e.x) -le 1) -and $m.y -eq $e.y -and $m.w -eq $e.w -and $m.h -eq $e.h
}

function Raiz-Em($x, $y) {
    $p = New-Object QaIlha+POINT; $p.X = $x; $p.Y = $y
    [QaIlha]::GetAncestor([QaIlha]::WindowFromPoint($p), 2)  # GA_ROOT
}

function Recortar($m, $nome) {
    try {
        $mx = 40; $x0 = [Math]::Max(0, $m.x - $mx); $y0 = [Math]::Max(0, $m.y)
        $bw = $m.w + 2 * $mx; $bh = $m.h + $mx
        $bmp = New-Object System.Drawing.Bitmap $bw, $bh
        $g = [System.Drawing.Graphics]::FromImage($bmp)
        $g.CopyFromScreen($x0, $y0, 0, 0, $bmp.Size)
        $z = New-Object System.Drawing.Bitmap ($bw * 4), ($bh * 4)
        $gz = [System.Drawing.Graphics]::FromImage($z)
        $gz.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::NearestNeighbor
        $gz.DrawImage($bmp, 0, 0, $z.Width, $z.Height)
        $z.Save((Join-Path $Saida $nome), [System.Drawing.Imaging.ImageFormat]::Png)
        $g.Dispose(); $gz.Dispose(); $bmp.Dispose(); $z.Dispose()
    } catch { Write-Host "recorte $nome falhou: $($_.Exception.Message)" }
}

function Esperar-Tamanho($h, $wLog, $hLog, $segundos) {
    $fim = (Get-Date).AddSeconds($segundos)
    do {
        $novo = Achar-Ilha
        if ($novo -ne [IntPtr]::Zero) { $h = $novo; $script:ilha = $novo }
        $m = Medir $h
        $e = Esperado $m $wLog $hLog
        if ($m.visivel -and (Bate $m $e)) { return @($m, $e, $true) }
        Start-Sleep -Milliseconds 200
    } while ((Get-Date) -lt $fim)
    return @($m, $e, $false)
}

function Alvo-Repouso($h, $m) {
    $canto = Raiz-Em ($m.x + 1) ($m.y + $m.h - 2)
    $centro = Raiz-Em ([int]($m.x + $m.w / 2)) ($m.y + 2)
    ($centro -eq $h) -and ($canto -eq $h)
}

function Conferir-Recorte($h, $m, $rotulo) {
    $cx = [int]($m.x + $m.w / 2)
    $cy = [int]($m.y + ($m.h / 2))
    $raizCentro = Raiz-Em $cx $cy
    $raizTopo = Raiz-Em $cx ($m.y + 1)
    $raizEsq = Raiz-Em ($m.x + 1) ($m.y + $m.h - 2)
    $raizDir = Raiz-Em ($m.x + $m.w - 2) ($m.y + $m.h - 2)
    Checar "$rotulo hit_test_capsula" (($raizCentro -eq $h) -and ($raizTopo -ne $h) -and ($raizEsq -ne $h) -and ($raizDir -ne $h)) ([ordered]@{
        centroEhIlha = ($raizCentro -eq $h); vaoTopoEhIlha = ($raizTopo -eq $h); cantoEsqEhIlha = ($raizEsq -eq $h); cantoDirEhIlha = ($raizDir -eq $h) })
}

$ilha = [IntPtr]::Zero
$fim = (Get-Date).AddSeconds(20)
while ($ilha -eq [IntPtr]::Zero -and (Get-Date) -lt $fim) { $ilha = Achar-Ilha; Start-Sleep -Milliseconds 250 }
Checar 'janela_da_ilha' ($ilha -ne [IntPtr]::Zero) ([ordered]@{ hwnd = [int64]$ilha })
if ($ilha -eq [IntPtr]::Zero) { $rel | ConvertTo-Json -Depth 6 | Set-Content (Join-Path $Saida 'ilha.json') -Encoding UTF8; exit 1 }
$ex = [QaIlha]::GetWindowLong($ilha, -20)
Checar 'estilos_sem_foco' ((($ex -band 0x08000000) -ne 0) -and (($ex -band 0x80) -ne 0) -and (($ex -band 0x8) -ne 0)) ([ordered]@{ exStyle = ('0x{0:X8}' -f $ex); noActivate = (($ex -band 0x08000000) -ne 0); toolWindow = (($ex -band 0x80) -ne 0); topMost = (($ex -band 0x8) -ne 0) })

# Bloco de Notas maximizado em primeiro plano: alvo do foco e janela que deve receber os cliques
# dos cantos. Tira o ponteiro de cima da ilha antes.
[void][QaIlha]::SetCursorPos(400, 400)
$np = Start-Process notepad.exe -PassThru
$hNp = [IntPtr]::Zero; $fim = (Get-Date).AddSeconds(15)
while ($hNp -eq [IntPtr]::Zero -and (Get-Date) -lt $fim) {
    Start-Sleep -Milliseconds 300
    # Notepad novo pode reaproveitar outro processo: procura por classe.
    foreach ($p in Get-Process notepad -ErrorAction SilentlyContinue) {
        foreach ($h in [QaIlha]::JanelasDo([uint32]$p.Id)) { if ([QaIlha]::IsWindowVisible($h) -and [QaIlha]::Classe($h) -eq 'Notepad') { $hNp = $h } }
    }
}
if ($hNp -ne [IntPtr]::Zero) {
    [void][QaIlha]::ShowWindow($hNp, 3)  # SW_MAXIMIZE
    [QaIlha]::keybd_event(0x12, 0, 0, [UIntPtr]::Zero); [QaIlha]::keybd_event(0x12, 0, 2, [UIntPtr]::Zero)  # Alt libera SetForegroundWindow
    [void][QaIlha]::SetForegroundWindow($hNp)
    Start-Sleep -Milliseconds 500
}
$fgInicial = [QaIlha]::GetForegroundWindow()
$focoPreparado = ($hNp -ne [IntPtr]::Zero) -and ($fgInicial -eq $hNp)
Checar 'foco_preparado_no_bloco_de_notas' $focoPreparado ([ordered]@{ notepad = [int64]$hNp; primeiroPlano = [int64]$fgInicial; classe = [QaIlha]::Classe($fgInicial) }) -Opcional
if (-not $focoPreparado) { $rel.limites += 'Não consegui pôr o Bloco de Notas em primeiro plano no runner; foco conferido contra a janela que estava em primeiro plano.' }
$alvoFoco = $fgInicial

# 1. Repouso (preferência pilulaSempreVisivel=1 gravada pelo executor).
$r = Esperar-Tamanho $ilha 72 14 15
Checar 'repouso_posicao_tamanho' $r[2] ([ordered]@{ medido = $r[0]; esperado = $r[1] })
Recortar $r[0] 'ilha-repouso.png'
if ($r[2]) { Checar 'repouso_alvo_retangular' (Alvo-Repouso $ilha $r[0]) $r[0] }
$rep = $r[0]

# 2. Hover: aceno e depois aberta (300 ms parado). Sem clicar, para não depender do tempo do aceno.
[void][QaIlha]::SetCursorPos([int]($rep.x + $rep.w / 2) - 2, $rep.y + 2)
Start-Sleep -Milliseconds 50
[QaIlha]::mouse_event(0x0001, 2, 0, 0, [UIntPtr]::Zero)  # MOUSEEVENTF_MOVE relativo: gera pointermove real
$a = Esperar-Tamanho $ilha 188 44 4
Checar 'hover_abre_aberta' $a[2] ([ordered]@{ medido = $a[0]; esperado = $a[1] })
Checar 'foco_apos_hover' ([QaIlha]::GetForegroundWindow() -eq $alvoFoco) ([ordered]@{ primeiroPlano = [int64][QaIlha]::GetForegroundWindow(); esperado = [int64]$alvoFoco })
Recortar $a[0] 'ilha-aberta.png'
if ($a[2]) {
    Start-Sleep -Milliseconds 500
    $ab = $a[0]
    Conferir-Recorte $ilha $ab 'aberta'
    # Clique numa área vazia dentro da curva (abaixo dos controles): não pode ativar a ilha.
    $ix = $ab.x + [int](32 * $ab.escala); $iy = $ab.y + [int](24 * $ab.escala)
    Checar 'ponto_de_clique_dentro_da_ilha' ((Raiz-Em $ix $iy) -eq $ilha) ([ordered]@{ x = $ix; y = $iy })
    [QaIlha]::Clicar($ix, $iy)
    Start-Sleep -Milliseconds 300
    Checar 'foco_apos_clique_na_ilha' ([QaIlha]::GetForegroundWindow() -eq $alvoFoco) ([ordered]@{ primeiroPlano = [int64][QaIlha]::GetForegroundWindow(); classe = [QaIlha]::Classe([QaIlha]::GetForegroundWindow()) })
    # Clique real no canto inferior esquerdo, fora da curva: vai para a janela de baixo.
    $cx = $ab.x + [int](6 * $ab.escala); $cy = $ab.y + $ab.h - [int](4 * $ab.escala)
    $raiz = Raiz-Em $cx $cy
    [QaIlha]::Clicar($cx, $cy)
    Start-Sleep -Milliseconds 300
    $fg = [QaIlha]::GetForegroundWindow()
    Checar 'clique_no_canto_atravessa' (($raiz -eq $hNp) -and ($fg -eq $hNp)) ([ordered]@{ janelaSobCanto = [QaIlha]::Classe($raiz); primeiroPlanoDepois = [QaIlha]::Classe($fg); ehBlocoDeNotas = ($raiz -eq $hNp) })
}
[void][QaIlha]::SetCursorPos(400, 400)
$v = Esperar-Tamanho $ilha 72 14 5
Checar 'volta_ao_repouso' $v[2] ([ordered]@{ medido = $v[0]; esperado = $v[1] })

# 3. Escalas possíveis no monitor do runner, trocadas em tempo de execução e restauradas.
try {
    $res = [QaIlha]::AumentarResolucao()
    $rel.resolucao = if ($res) { "trocada para $res (código 0 = ok)" } else { 'sem modo maior que o atual' }
    Start-Sleep -Milliseconds 2000
    $info = [QaIlha]::LerEscala()
    $orig = $info[1]
    $rel.escalaOriginal = [ordered]@{ recomendada = $info[0]; atual = $info[1]; maxima = $info[3] }
    $possiveis = @($Escalas | Where-Object { $_ -le $info[3] })
    $rel.limites += "Escalas aceitas pelo monitor do runner: até $($info[3])% (pedidas: $($Escalas -join ', '))."
    foreach ($pct in $possiveis) {
        $rc = [QaIlha]::DefinirEscala($pct)
        Start-Sleep -Milliseconds 1500
        $m = Esperar-Tamanho $ilha 72 14 6
        $aplicada = [Math]::Round($m[0].escala * 100)
        # Fonte independente: o que o sistema diz da escala e do DPI efetivo do monitor da ilha.
        $sistema = [QaIlha]::LerEscala()[1]
        $dpiMon = [Math]::Round([QaIlha]::DpiDoMonitor($ilha) / 96.0 * 100)
        $noSistema = ($sistema -eq $pct) -and ($dpiMon -eq $pct)
        $item = [ordered]@{ pedida = $pct; codigo = $rc; escalaDoSistema = $sistema; dpiDoMonitor = $dpiMon; aplicadaNoSistema = $noSistema
            aplicadaNaIlha = $aplicada; medido = $m[0]; esperado = $m[1]; ok = ($m[2] -and $aplicada -eq $pct) }
        if ($item.ok) {
            $item.alvoCompleto = Alvo-Repouso $ilha $m[0]
            $item.ok = $item.alvoCompleto
        }
        $rel.escalas += $item
        Recortar $m[0] ("ilha-repouso-{0}.png" -f $pct)
        if (-not $noSistema) {
            # O sistema não aplicou a escala nesta sessão: limite do runner, não falha da ilha.
            $rel.limites += "Escala $pct% não aplicada pelo sistema na sessão (sistema=$sistema%, monitor=$dpiMon%)."
        } elseif (-not $item.ok) { $falhas++ }
        Write-Host ("escala {0}%: código {1}, sistema {2}%, monitor {3}%, ilha {4}%, ok={5}" -f $pct, $rc, $sistema, $dpiMon, $aplicada, $item.ok)
        # O patch limita a 2 recriações a cada 20 s (antilaço); troca de escala espaçada como um usuário.
        Start-Sleep -Seconds 11
    }
    # Rajada (patch v2 do Dev Front): trocas a cada ~6,5 s até a 3ª cair no limite antilaço de 2
    # recriações em 20 s. Passa se o app registrar "ilha: retentativa: ... -> Recriar" e a ilha ficar
    # na escala final sem nova troca.
    $rajada = @(125, 150, 175) | Where-Object { $_ -le $info[3] }
    if ($rajada.Count -eq 3) {
        Start-Sleep -Seconds 21  # zera a janela do limite
        $logApp = if ($env:DITASE_RAIZ) { Join-Path $env:DITASE_RAIZ 'ditado.log' } else { '' }
        $linhasAntes = if ($logApp -and (Test-Path $logApp)) { @(Get-Content -LiteralPath $logApp).Count } else { 0 }
        $t0 = Get-Date
        foreach ($pct in $rajada) { [void][QaIlha]::DefinirEscala($pct); Start-Sleep -Milliseconds 6500 }
        Start-Sleep -Seconds 22  # o limite libera ~20 s depois da 1ª recriação; sem nova troca
        $m = Esperar-Tamanho $ilha 72 14 8
        $alvoCompleto = Alvo-Repouso $ilha $m[0]
        $novas = if ($logApp -and (Test-Path $logApp)) { @(Get-Content -LiteralPath $logApp | Select-Object -Skip $linhasAntes | Where-Object { $_ -match 'ilha:' } | ForEach-Object { [string]$_ }) } else { @() }
        $retentativa = @($novas | Where-Object { $_ -match 'ilha: retentativa:.*-> Recriar' })
        $limite = @($novas | Where-Object { $_ -match 'limite atingido' })
        $escFinal = [Math]::Round($m[0].escala * 100)
        $rel.rajada = [ordered]@{ duracaoS = [int]((Get-Date) - $t0).TotalSeconds; medido = $m[0]; esperado = $m[1]; alvoCompleto = $alvoCompleto
            limiteAtingido = $limite.Count; retentativas = $retentativa.Count; linhasIlha = $novas }
        Checar 'rajada_limite_e_retentativa' (($limite.Count -ge 1) -and ($retentativa.Count -ge 1) -and $m[2] -and ($escFinal -eq 175) -and $alvoCompleto) ([ordered]@{
            limiteAtingido = $limite.Count; retentativas = $retentativa.Count; escalaFinal = $escFinal; medido = $m[0]; esperado = $m[1]; recorteOk = ($alvoCompleto) })
        Recortar $m[0] 'ilha-rajada-175.png'
    } else {
        $rel.limites += 'Rajada não executada: o monitor não aceita 125, 150 e 175%.'
    }

    # Abertura já em cada escala acima de 100% (caso mais comum): reinicia o app instalado.
    $altas = @($possiveis | Where-Object { $_ -gt 100 })
    if ($Executavel) {
        foreach ($pctAlta in $altas) {
            [void][QaIlha]::DefinirEscala($pctAlta)
            Start-Sleep -Milliseconds 1500
            Stop-Process -Id $script:ProcessoApp -Force -ErrorAction SilentlyContinue
            Start-Sleep -Seconds 2
            $novo = Start-Process -FilePath $Executavel -PassThru
            $script:ProcessoApp = $novo.Id
            $ilha = [IntPtr]::Zero; $fimN = (Get-Date).AddSeconds(30)
            while ($ilha -eq [IntPtr]::Zero -and (Get-Date) -lt $fimN) { Start-Sleep -Milliseconds 300; $ilha = Achar-Ilha }
            if ($ilha -eq [IntPtr]::Zero) {
                Checar "abertura_em_$pctAlta" $false ([ordered]@{ motivo = 'ilha não apareceu após reiniciar' })
                continue
            }
            $m = Esperar-Tamanho $ilha 72 14 20
            $alvoCompleto = Alvo-Repouso $ilha $m[0]
            $esc = [Math]::Round($m[0].escala * 100)
            Checar "abertura_em_$pctAlta" ($m[2] -and $esc -eq $pctAlta -and $alvoCompleto) ([ordered]@{ medido = $m[0]; esperado = $m[1]; recorteOk = ($alvoCompleto) })
            Recortar $m[0] ("ilha-abertura-{0}.png" -f $pctAlta)
        }
        Stop-Process -Id $script:ProcessoApp -Force -ErrorAction SilentlyContinue
    }
    [void][QaIlha]::DefinirEscala($orig)
    if ($res) { [void][QaIlha]::RestaurarResolucao() }
    Start-Sleep -Milliseconds 1000
} catch {
    $rel.limites += "Troca de escala indisponível no runner: $($_.Exception.Message)"
}

try { if ($np -and -not $np.HasExited) { Stop-Process -Id $np.Id -Force } } catch {}
Get-Process notepad -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
$rel.janelasNoActivate = @(Listar-Candidatas | ForEach-Object { $_ | Select-Object @{n='hwnd';e={[int64]$_.hwnd}}, classe, exStyle, visivel, regiao, x, y, w, h })
$rel.falhas = $falhas
$rel | ConvertTo-Json -Depth 6 | Set-Content (Join-Path $Saida 'ilha.json') -Encoding UTF8
exit ([int]($falhas -gt 0))
