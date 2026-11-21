//! Pílula (aba na borda) sem roubar o foco:
//! - `WS_EX_NOACTIVATE` faz o clique não ativar a janela (o texto continua indo para o app em foco).
//! - `WS_EX_TOOLWINDOW` tira da barra de tarefas e do Alt+Tab.
//! - `SetWindowPos` com `SWP_NOACTIVATE | SWP_NOZORDER` posiciona, redimensiona e mostra numa
//!   chamada só, sem ativar a janela nem mostrar quadro deslocado.
//! - `SHQueryUserNotificationState` detecta tela cheia / apresentação na troca de janela em foco
//!   (`EVENT_SYSTEM_FOREGROUND`), sem consulta periódica.
use std::sync::{Arc, OnceLock};

use windows::Win32::{
    Foundation::{HWND, POINT, RECT},
    UI::{
        Accessibility::{SetWinEventHook, HWINEVENTHOOK},
        Shell::{
            SHQueryUserNotificationState, QUNS_BUSY, QUNS_PRESENTATION_MODE,
            QUNS_RUNNING_D3D_FULL_SCREEN,
        },
        WindowsAndMessaging::{
            DispatchMessageW, GetCursorPos, GetMessageW, GetWindowLongPtrW, GetWindowRect,
            SetWindowLongPtrW, SetWindowPos, TranslateMessage, EVENT_SYSTEM_FOREGROUND,
            GWL_EXSTYLE, MSG, SWP_HIDEWINDOW, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
            SWP_SHOWWINDOW, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WS_EX_NOACTIVATE,
            WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
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

/// Posiciona, redimensiona e opcionalmente mostra a aba numa única chamada nativa,
/// nunca ativando a janela (`SWP_NOACTIVATE`).
pub fn posicionar_sem_ativar(hwnd: isize, x: i32, y: i32, w: i32, h: i32, mostrar: bool) {
    unsafe {
        let mut flags = SWP_NOACTIVATE | SWP_NOZORDER;
        if mostrar {
            flags |= SWP_SHOWWINDOW;
        }
        let _ = SetWindowPos(HWND(hwnd as *mut _), None, x, y, w, h, flags);
    }
}

/// Esconde a aba sem mexer no foco.
pub fn esconder(hwnd: isize) {
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
}

/// Observa trocas de janela em primeiro plano (`EVENT_SYSTEM_FOREGROUND`) e informa se há tela cheia.
/// Roda em thread própria com laço de mensagens Win32; zero polling em repouso.
pub fn observar_tela_cheia(cb: Arc<dyn Fn(bool) + Send + Sync>) {
    if AO_MUDAR_FOCO.set(cb).is_err() {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("ditase.telacheia".into())
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
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        });
}
