//! Ilha no topo da tela sem roubar o foco:
//! - `WS_EX_NOACTIVATE` faz o clique não ativar a janela (o texto continua indo para o app em foco).
//! - `WS_EX_TOOLWINDOW` tira da barra de tarefas e do Alt+Tab.
//! - `SetWindowPos` com `SWP_NOACTIVATE | SWP_NOZORDER` posiciona, redimensiona e mostra numa
//!   chamada só, sem ativar a janela nem mostrar quadro deslocado.
//! - `SetWindowRgn` com uma `CreateEllipticRgn` recorta a janela na meia elipse da ilha: o clique
//!   fora da curva atravessa para a janela de baixo (pixel transparente do WebView2 ainda pegaria).
//! - Tela que muda com o app aberto (resolução, escala, área de trabalho): uma janela oculta recebe
//!   `WM_DISPLAYCHANGE` e `WM_SETTINGCHANGE`, e um gancho `EVENT_OBJECT_LOCATIONCHANGE` só do nosso
//!   processo avisa quando alguém (o tao no `WM_DPICHANGED`, por exemplo) move a ilha para fora do
//!   lugar que pedimos. Nada disso faz consulta periódica.
//! - `SHQueryUserNotificationState` detecta tela cheia / apresentação na troca de janela em foco
//!   (`EVENT_SYSTEM_FOREGROUND`), sem consulta periódica.
use std::sync::{
    atomic::{AtomicIsize, Ordering},
    Arc, Mutex, OnceLock,
};

use windows::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::Gdi::{
        CreateEllipticRgn, DeleteObject, MonitorFromWindow, SetWindowRgn, HGDIOBJ,
        MONITOR_DEFAULTTOPRIMARY,
    },
    System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentProcessId},
    UI::{
        Accessibility::{SetWinEventHook, HWINEVENTHOOK},
        HiDpi::{GetDpiForMonitor, GetDpiForWindow, MDT_EFFECTIVE_DPI},
        Shell::{
            SHQueryUserNotificationState, QUNS_BUSY, QUNS_PRESENTATION_MODE,
            QUNS_RUNNING_D3D_FULL_SCREEN,
        },
        WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DispatchMessageW, GetCursorPos, GetMessageW,
            GetWindowLongPtrW, GetWindowRect, RegisterClassW, SetWindowLongPtrW, SetWindowPos,
            TranslateMessage, EVENT_OBJECT_LOCATIONCHANGE, EVENT_SYSTEM_FOREGROUND, GWL_EXSTYLE,
            MSG, OBJID_WINDOW, SWP_HIDEWINDOW, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
            SWP_NOZORDER, SWP_SHOWWINDOW, WINDOW_EX_STYLE, WINEVENT_OUTOFCONTEXT,
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

/// Posiciona, redimensiona e opcionalmente mostra a ilha numa única chamada nativa,
/// nunca ativando a janela (`SWP_NOACTIVATE`).
pub fn posicionar_sem_ativar(hwnd: isize, x: i32, y: i32, w: i32, h: i32, mostrar: bool) {
    if hwnd == ILHA.load(Ordering::Acquire) {
        *ESPERADO.lock().unwrap() = Some((x, y, w, h));
    }
    unsafe {
        let mut flags = SWP_NOACTIVATE | SWP_NOZORDER;
        if mostrar {
            flags |= SWP_SHOWWINDOW;
        }
        let _ = SetWindowPos(HWND(hwnd as *mut _), None, x, y, w, h, flags);
    }
}

/// Recorta a janela na elipse de retângulo envolvente `(l, t, r, b)` (pixels físicos relativos à
/// janela; ver `pilula::regiao_meia_elipse`). Em sucesso o sistema passa a ser dono da região; em
/// falha ela é apagada aqui e a janela fica retangular (só perde o clique atravessando os cantos).
pub fn recortar_elipse(hwnd: isize, l: i32, t: i32, r: i32, b: i32) {
    unsafe {
        let rgn = CreateEllipticRgn(l, t, r, b);
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

/// Verdadeiro quando há aplicativo em tela cheia ou modo de apresentação.
pub fn tela_cheia_ativa() -> bool {
    unsafe {
        match SHQueryUserNotificationState() {
            Ok(estado) => {
                estado == QUNS_BUSY
                    || estado == QUNS_RUNNING_D3D_FULL_SCREEN
                    || estado == QUNS_PRESENTATION_MODE
            }
            Err(_) => false,
        }
    }
}

/// Verdadeiro quando o ponteiro do mouse está fora do retângulo da janela.
pub fn cursor_fora_da_janela(hwnd: isize) -> bool {
    unsafe {
        let mut pt = POINT::default();
        let mut rc = RECT::default();
        if GetCursorPos(&mut pt).is_err() || GetWindowRect(HWND(hwnd as *mut _), &mut rc).is_err() {
            return false;
        }
        pt.x < rc.left || pt.x >= rc.right || pt.y < rc.top || pt.y >= rc.bottom
    }
}

static AO_MUDAR_FOCO: OnceLock<Arc<dyn Fn(bool) + Send + Sync>> = OnceLock::new();
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
    if let Some(cb) = AO_MUDAR_FOCO.get() {
        cb(tela_cheia_ativa());
    }
    // Rede de segurança sem consulta periódica: ao voltar das Configurações (ou de qualquer
    // janela), a ilha confere se a tela mudou.
    avisar_tela("primeiro-plano");
}

/// Deriva: a ilha saiu do retângulo que pedimos (ex.: o tao reposiciona depois do
/// `WM_DPICHANGED`, sobrescrevendo o nosso `SetWindowPos`).
unsafe extern "system" fn gancho_posicao(
    _hook: HWINEVENTHOOK,
    _event: u32,
    hwnd: HWND,
    id_object: i32,
    _id_child: i32,
    _event_thread: u32,
    _event_time: u32,
) {
    let ilha = ILHA.load(Ordering::Acquire);
    if ilha == 0 || hwnd.0 as isize != ilha || id_object != OBJID_WINDOW.0 {
        return;
    }
    let Some(esperado) = retangulo_esperado() else {
        return;
    };
    if retangulo(ilha).is_some_and(|real| real != esperado) {
        avisar_tela("deriva");
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

/// Observa trocas de janela em primeiro plano (`EVENT_SYSTEM_FOREGROUND`, para tela cheia) e as
/// mudanças de tela (resolução, escala, área de trabalho e deriva da ilha). Roda em thread própria
/// com laço de mensagens Win32; zero consulta periódica em repouso.
pub fn observar(
    tela_cheia: Arc<dyn Fn(bool) + Send + Sync>,
    tela: Arc<dyn Fn(&'static str) + Send + Sync>,
) {
    if AO_MUDAR_FOCO.set(tela_cheia).is_err() {
        return;
    }
    let _ = AO_MUDAR_TELA.set(tela);
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
                WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
            );
            if hook.0.is_null() {
                return;
            }
            // Só janelas do nosso processo (as do WebView2 são de outro processo).
            let _ = SetWinEventHook(
                EVENT_OBJECT_LOCATIONCHANGE,
                EVENT_OBJECT_LOCATIONCHANGE,
                None,
                Some(gancho_posicao),
                GetCurrentProcessId(),
                0,
                WINEVENT_OUTOFCONTEXT,
            );
            criar_ouvinte_de_tela();
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        });
}
