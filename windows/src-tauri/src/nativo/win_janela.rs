//! Pílula sem roubar o foco: `WS_EX_NOACTIVATE` faz o clique não ativar a janela (o texto
//! continua indo para o app em que a pessoa está), `WS_EX_TOOLWINDOW` tira da barra de tarefas
//! e do Alt+Tab.
use windows::Win32::{
    Foundation::HWND,
    UI::WindowsAndMessaging::{GetWindowLongPtrW, SetWindowLongPtrW, GWL_EXSTYLE, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST},
};

pub fn sem_foco(hwnd: isize) {
    unsafe {
        let h = HWND(hwnd as *mut _);
        let atual = GetWindowLongPtrW(h, GWL_EXSTYLE);
        let novo = atual | (WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0 | WS_EX_TOPMOST.0) as isize;
        SetWindowLongPtrW(h, GWL_EXSTYLE, novo);
    }
}
