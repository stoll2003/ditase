//! Ilha no topo da tela sem roubar o foco:
//! - `WS_EX_NOACTIVATE` faz o clique não ativar a janela (o texto continua indo para o app em foco).
//! - `WS_EX_TOOLWINDOW` tira da barra de tarefas e do Alt+Tab.
//! - `SetWindowPos` com `HWND_TOPMOST` e `SWP_NOACTIVATE` mostra a ilha acima das outras
//!   janelas sem ativá-la. Na troca de foco, a ordem é reafirmada sem mover ou mostrar a janela.
//! - `SetWindowRgn` com uma `CreateRoundRectRgn` recorta a janela na cápsula da ilha (1 px físico
//!   maior que o desenho): o clique nas quinas e no vão de cima atravessa para a janela de baixo
//!   (pixel transparente do WebView2 ainda pegaria).
//! - Tela que muda com o app aberto (resolução, escala, área de trabalho): uma janela oculta recebe
//!   `WM_DISPLAYCHANGE` e `WM_SETTINGCHANGE`, e um gancho `EVENT_OBJECT_LOCATIONCHANGE` só do nosso
//!   processo avisa quando alguém (o tao no `WM_DPICHANGED`, por exemplo) move a ilha para fora do
//!   lugar que pedimos. Nada disso faz consulta periódica.
use std::sync::{
    atomic::{AtomicIsize, Ordering},
    Arc, Mutex, OnceLock,
};

use windows::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::Gdi::{
        CreateRectRgn, CreateRoundRectRgn, DeleteObject, GetWindowRgn, MonitorFromWindow,
        PtInRegion, SetWindowRgn, COMPLEXREGION, HGDIOBJ, MONITOR_DEFAULTTOPRIMARY,
    },
    System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentProcessId},
    UI::{
        Accessibility::{SetWinEventHook, HWINEVENTHOOK},
        HiDpi::{GetDpiForMonitor, GetDpiForWindow, MDT_EFFECTIVE_DPI},
        WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DispatchMessageW, GetCursorPos, GetDesktopWindow,
            GetForegroundWindow, GetMessageW, GetWindow, GetWindowLongPtrW, GetWindowRect,
            IsWindowVisible, RegisterClassW, SetWindowLongPtrW, SetWindowPos, TranslateMessage,
            EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_REORDER, EVENT_OBJECT_SHOW,
            EVENT_SYSTEM_FOREGROUND, GWL_EXSTYLE, GW_HWNDPREV, HWND_TOPMOST, MSG, OBJID_CLIENT,
            OBJID_WINDOW, SWP_ASYNCWINDOWPOS, SWP_HIDEWINDOW, SWP_NOACTIVATE, SWP_NOMOVE,
            SWP_NOSIZE, SWP_NOZORDER, SWP_SHOWWINDOW, WINDOW_EX_STYLE, WINEVENT_OUTOFCONTEXT,
            WINEVENT_SKIPOWNPROCESS, WM_DISPLAYCHANGE, WM_DPICHANGED, WM_SETTINGCHANGE, WNDCLASSW,
            WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
        },
    },
};

pub fn sem_foco(hwnd: isize) {
    unsafe {
        let h = HWND(hwnd as *mut _);
        let atual = GetWindowLongPtrW(h, GWL_EXSTYLE);
        let novo = atual | (WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0 | WS_EX_TOPMOST.0) as isize;
        SetWindowLongPtrW(h, GWL_EXSTYLE, novo);
    }
}

/// Janela da ilha vigiada pelo gancho de deriva, e o retângulo que nós pedimos para ela.
static ILHA: AtomicIsize = AtomicIsize::new(0);
static ESPERADO: Mutex<Option<(i32, i32, i32, i32)>> = Mutex::new(None);

/// Registra a janela da ilha (recriada também passa por aqui).
pub fn registrar_ilha(hwnd: isize) {
    ILHA.store(hwnd, Ordering::Release);
    *ESPERADO.lock().unwrap() = None;
}

/// Retângulo que pedimos por último (`None` com a ilha escondida).
pub fn retangulo_esperado() -> Option<(i32, i32, i32, i32)> {
    *ESPERADO.lock().unwrap()
}

/// Retângulo real da janela em pixels físicos.
pub fn retangulo(hwnd: isize) -> Option<(i32, i32, i32, i32)> {
    unsafe {
        let mut rc = RECT::default();
        GetWindowRect(HWND(hwnd as *mut _), &mut rc).ok()?;
        Some((rc.left, rc.top, rc.right - rc.left, rc.bottom - rc.top))
    }
}

/// Mantém a moldura e seus botões nativos. Versões antigas do Windows mantêm as cores do sistema
/// quando o DWM não oferece estes atributos.
pub fn pintar_barra_de_titulo(
    janela: &tauri::WebviewWindow,
    fundo: tauri::window::Color,
    texto: tauri::window::Color,
) {
    use windows::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMSBT_NONE, DWMWA_CAPTION_COLOR, DWMWA_SYSTEMBACKDROP_TYPE,
        DWMWA_TEXT_COLOR,
    };

    let Ok(hwnd) = janela.hwnd() else { return };
    let colorref = |cor: tauri::window::Color| u32::from_le_bytes([cor.0, cor.1, cor.2, 0]);
    let atributos = [
        (DWMWA_SYSTEMBACKDROP_TYPE, DWMSBT_NONE.0 as u32),
        (DWMWA_CAPTION_COLOR, colorref(fundo)),
        (DWMWA_TEXT_COLOR, colorref(texto)),
    ];
    for (atributo, valor) in atributos {
        let _ = unsafe {
            DwmSetWindowAttribute(
                HWND(hwnd.0),
                atributo,
                (&valor as *const u32).cast(),
                size_of::<u32>() as u32,
            )
        };
    }
}

/// DPI que a janela usa de fato (é a escala em que o WebView2 desenha).
pub fn dpi_da_janela(hwnd: isize) -> u32 {
    unsafe { GetDpiForWindow(HWND(hwnd as *mut _)) }
}

/// DPI efetivo do monitor em que a janela está (ou do principal).
pub fn dpi_do_monitor(hwnd: isize) -> u32 {
    unsafe {
        let monitor = MonitorFromWindow(HWND(hwnd as *mut _), MONITOR_DEFAULTTOPRIMARY);
        let (mut x, mut y) = (0u32, 0u32);
        if GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut x, &mut y).is_ok() {
            x
        } else {
            0
        }
    }
}

/// Tipo da região da janela (`GetWindowRgn`: 0 sem região, 1 vazia, 2 retângulo, 3 complexa) e
/// se o canto inferior esquerdo está fora dela (como deve estar na cápsula).
pub fn estado_da_regiao(hwnd: isize) -> (i32, bool) {
    unsafe {
        let rgn = CreateRectRgn(0, 0, 0, 0);
        if rgn.is_invalid() {
            return (0, false);
        }
        let tipo = GetWindowRgn(HWND(hwnd as *mut _), rgn).0;
        let canto_fora = match retangulo(hwnd) {
            Some((_, _, _, h)) if tipo != 0 => !PtInRegion(rgn, 0, h - 1).as_bool(),
            _ => false,
        };
        let _ = DeleteObject(HGDIOBJ(rgn.0));
        (tipo, canto_fora)
    }
}

/// Verdadeiro quando a região é a cápsula: complexa e com o canto inferior esquerdo fora.
pub fn regiao_recortada(hwnd: isize) -> bool {
    let (tipo, canto_fora) = estado_da_regiao(hwnd);
    tipo == COMPLEXREGION.0 && canto_fora
}

/// Posiciona, redimensiona e opcionalmente mostra a ilha numa única chamada nativa,
/// nunca ativando a janela (`SWP_NOACTIVATE`).
pub fn posicionar_sem_ativar(hwnd: isize, x: i32, y: i32, w: i32, h: i32, mostrar: bool) {
    if hwnd == ILHA.load(Ordering::Acquire) {
        *ESPERADO.lock().unwrap() = Some((x, y, w, h));
    }
    unsafe {
        let mut flags = SWP_NOACTIVATE;
        if mostrar {
            flags |= SWP_SHOWWINDOW;
        }
        let _ = SetWindowPos(HWND(hwnd as *mut _), Some(HWND_TOPMOST), x, y, w, h, flags);
    }
}

/// Recorta a janela na cápsula `(esquerda, topo, direita, base, diâmetro)` em pixels físicos
/// relativos à janela (ver `pilula::regiao_capsula`), ou tira o recorte (`None`: a janela inteira
/// pega o clique, na barrinha e durante a transição). Em sucesso o sistema passa a ser dono da
/// região; em falha ela é apagada aqui.
pub fn recortar(hwnd: isize, regiao: Option<(i32, i32, i32, i32, i32)>) {
    unsafe {
        let Some((l, t, r, b, d)) = regiao else {
            let _ = SetWindowRgn(HWND(hwnd as *mut _), None, true);
            return;
        };
        let rgn = CreateRoundRectRgn(l, t, r, b, d, d);
        if rgn.is_invalid() {
            return;
        }
        if SetWindowRgn(HWND(hwnd as *mut _), Some(rgn), true) == 0 {
            let _ = DeleteObject(HGDIOBJ(rgn.0));
        }
    }
}

/// Esconde a ilha sem mexer no foco.
pub fn esconder(hwnd: isize) {
    if hwnd == ILHA.load(Ordering::Acquire) {
        *ESPERADO.lock().unwrap() = None;
    }
    unsafe {
        let _ = SetWindowPos(
            HWND(hwnd as *mut _),
            None,
            0,
            0,
            0,
            0,
            SWP_HIDEWINDOW | SWP_NOACTIVATE | SWP_NOZORDER | SWP_NOMOVE | SWP_NOSIZE,
        );
    }
}

/// Reafirma a ordem de uma ilha visível, inclusive diante de outra janela topmost.
pub fn reafirmar_topo(hwnd: isize) {
    unsafe {
        let janela = HWND(hwnd as *mut _);
        if !IsWindowVisible(janela).as_bool() {
            return;
        }
        let _ = SetWindowPos(
            janela,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE | SWP_ASYNCWINDOWPOS,
        );
    }
}

/// O app em foco pode virar uma janela sem borda sem receber outro evento de foco.
fn conferir_ordem(ilha: HWND, frente: HWND) {
    if frente.is_invalid() || frente == ilha {
        return;
    }
    let Some((x, y, w, h)) = retangulo(ilha.0 as isize) else {
        return;
    };
    let Some((fx, fy, fw, fh)) = retangulo(frente.0 as isize) else {
        return;
    };
    if x >= fx + fw || fx >= x + w || y >= fy + fh || fy >= y + h {
        return;
    }
    // Limita a caminhada porque outras janelas podem mudar de posição durante a consulta.
    let mut acima = ilha;
    for _ in 0..128 {
        let Ok(anterior) = (unsafe { GetWindow(acima, GW_HWNDPREV) }) else {
            break;
        };
        if anterior == frente {
            reafirmar_topo(ilha.0 as isize);
            break;
        }
        acima = anterior;
    }
}

/// Verdadeiro quando o ponteiro está fora do retângulo da janela ampliado em `lado` px para os
/// lados e `baixo` px para baixo; com `ate_o_topo`, a zona sobe até a borda de cima da tela.
pub fn cursor_fora_da_zona(hwnd: isize, lado: i32, baixo: i32, ate_o_topo: bool) -> bool {
    unsafe {
        let mut pt = POINT::default();
        let mut rc = RECT::default();
        if GetCursorPos(&mut pt).is_err() || GetWindowRect(HWND(hwnd as *mut _), &mut rc).is_err() {
            return false;
        }
        let topo = if ate_o_topo { i32::MIN } else { rc.top };
        pt.x < rc.left - lado || pt.x >= rc.right + lado || pt.y < topo || pt.y >= rc.bottom + baixo
    }
}

static AO_MUDAR_TELA: OnceLock<Arc<dyn Fn(&'static str) + Send + Sync>> = OnceLock::new();

fn avisar_tela(motivo: &'static str) {
    if let Some(cb) = AO_MUDAR_TELA.get() {
        cb(motivo);
    }
}

unsafe extern "system" fn gancho_foco(
    _hook: HWINEVENTHOOK,
    _event: u32,
    _hwnd: HWND,
    _id_object: i32,
    _id_child: i32,
    _event_thread: u32,
    _event_time: u32,
) {
    let ilha = ILHA.load(Ordering::Acquire);
    if ilha != 0 {
        reafirmar_topo(ilha);
    }
    // Rede de segurança sem consulta periódica: ao voltar das Configurações (ou de qualquer
    // janela), a ilha confere se a tela mudou.
    avisar_tela("primeiro-plano");
}

/// Uma mudança do tao no DPI pode mover a ilha depois do nosso SetWindowPos.
unsafe extern "system" fn gancho_posicao_ilha(
    _hook: HWINEVENTHOOK,
    _evento: u32,
    hwnd: HWND,
    id_object: i32,
    id_child: i32,
    _event_thread: u32,
    _event_time: u32,
) {
    let ilha = ILHA.load(Ordering::Acquire);
    if ilha == 0 || hwnd.0 as isize != ilha || id_object != OBJID_WINDOW.0 || id_child != 0 {
        return;
    }
    let Some(esperado) = retangulo_esperado() else {
        return;
    };
    if retangulo(ilha).is_some_and(|real| real != esperado) {
        avisar_tela("deriva");
    }
}

unsafe extern "system" fn gancho_janela_externa(
    _hook: HWINEVENTHOOK,
    evento: u32,
    hwnd: HWND,
    id_object: i32,
    id_child: i32,
    _event_thread: u32,
    _event_time: u32,
) {
    let janela = id_object == OBJID_WINDOW.0;
    let ordem = evento == EVENT_OBJECT_REORDER;
    if id_child != 0 || !(janela || (ordem && id_object == OBJID_CLIENT.0)) {
        return;
    }
    let ilha = ILHA.load(Ordering::Acquire);
    if ilha == 0 {
        return;
    }
    let frente = unsafe { GetForegroundWindow() };
    if hwnd == frente || (ordem && hwnd == unsafe { GetDesktopWindow() }) {
        conferir_ordem(HWND(ilha as *mut _), frente);
    }
}

/// Janela oculta (não é `HWND_MESSAGE`, que não recebe difusão) que ouve as mudanças de tela.
unsafe extern "system" fn proc_tela(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_DISPLAYCHANGE => avisar_tela("resolucao"),
        WM_SETTINGCHANGE => avisar_tela("configuracao"),
        WM_DPICHANGED => avisar_tela("dpi"),
        _ => {}
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

unsafe fn criar_ouvinte_de_tela() {
    let instancia = GetModuleHandleW(None).unwrap_or_default();
    let classe = windows::core::w!("DitaseOuvinteDeTela");
    let wc = WNDCLASSW {
        lpfnWndProc: Some(proc_tela),
        hInstance: instancia.into(),
        lpszClassName: classe,
        ..Default::default()
    };
    RegisterClassW(&wc);
    // WS_POPUP oculta e sem dono: recebe WM_DISPLAYCHANGE e WM_SETTINGCHANGE por difusão.
    let _ = CreateWindowExW(
        WINDOW_EX_STYLE(WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0),
        classe,
        windows::core::w!(""),
        WS_POPUP,
        0,
        0,
        0,
        0,
        None,
        None,
        Some(instancia.into()),
        None,
    );
}

/// Observa trocas de janela em primeiro plano e as
/// mudanças de tela (resolução, escala, área de trabalho e deriva da ilha). Roda em thread própria
/// com laço de mensagens Win32; zero consulta periódica em repouso.
pub fn observar(tela: Arc<dyn Fn(&'static str) + Send + Sync>) {
    if AO_MUDAR_TELA.set(tela).is_err() {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("ditase.tela".into())
        .spawn(|| unsafe {
            let hook = SetWinEventHook(
                EVENT_SYSTEM_FOREGROUND,
                EVENT_SYSTEM_FOREGROUND,
                None,
                Some(gancho_foco),
                0,
                0,
                WINEVENT_OUTOFCONTEXT,
            );
            if hook.0.is_null() {
                return;
            }
            let _ = SetWinEventHook(
                EVENT_OBJECT_LOCATIONCHANGE,
                EVENT_OBJECT_LOCATIONCHANGE,
                None,
                Some(gancho_posicao_ilha),
                GetCurrentProcessId(),
                0,
                WINEVENT_OUTOFCONTEXT,
            );
            for evento in [
                EVENT_OBJECT_LOCATIONCHANGE,
                EVENT_OBJECT_SHOW,
                EVENT_OBJECT_REORDER,
            ] {
                let _ = SetWinEventHook(
                    evento,
                    evento,
                    None,
                    Some(gancho_janela_externa),
                    0,
                    0,
                    WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
                );
            }
            criar_ouvinte_de_tela();
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        });
}
