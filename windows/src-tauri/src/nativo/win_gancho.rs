//! Gancho de teclado de baixo nível (`WH_KEYBOARD_LL`), porta do event tap de `Atalho.swift`.
//!
//! Privacidade e antivírus: o gancho olha só a tecla do atalho, Esc, Espaço (com o atalho
//! segurado) e Ctrl+Alt+V. Qualquer outra tecla só vira "outra tecla" enquanto o atalho está
//! segurado, sem código nem texto. Nada é gravado, nada sai da thread do gancho além disso.
//! Teclas injetadas (inclusive o Ctrl+V da nossa colagem) são ignoradas.
use std::{
    collections::HashSet,
    sync::{atomic::{AtomicBool, Ordering}, mpsc::Sender, Arc, Mutex, OnceLock},
    thread,
    time::Instant,
};

use windows::Win32::{
    Foundation::{LPARAM, LRESULT, WPARAM},
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        Input::KeyboardAndMouse::GetAsyncKeyState,
        WindowsAndMessaging::{
            CallNextHookEx, DispatchMessageW, GetMessageW, SetWindowsHookExW, TranslateMessage, HC_ACTION, KBDLLHOOKSTRUCT,
            LLKHF_INJECTED, MSG, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
        },
    },
};

use crate::atalho::{Entrada, Visao};

struct Estado {
    visao: Arc<Mutex<Visao>>,
    emissor: Mutex<Sender<Entrada>>,
    relogio: Instant,
    apertada: AtomicBool,
    /// Teclas cujo aperto engolimos: o soltar também é engolido, mesmo que o modo tenha mudado.
    consumidas: Mutex<HashSet<u32>>,
    /// Teclas descidas, só para reconhecer a repetição automática. Só códigos, e só enquanto descidas.
    abaixadas: Mutex<HashSet<u32>>,
}

static ESTADO: OnceLock<Estado> = OnceLock::new();

const VK_ESCAPE: u32 = 0x1B;
const VK_SPACE: u32 = 0x20;
const VK_V: u32 = 0x56;
const VK_CONTROL: i32 = 0x11;
const VK_MENU: i32 = 0x12;
const VK_SHIFT: i32 = 0x10;
const VK_LWIN: i32 = 0x5B;
const VK_RWIN: i32 = 0x5C;

fn modificador(vk: u32) -> bool { matches!(vk, 0x10 | 0x11 | 0x12 | 0xA0..=0xA5 | 0x5B | 0x5C) }
fn descida(vk: i32) -> bool { unsafe { GetAsyncKeyState(vk) as u16 & 0x8000 != 0 } }

/// Instala o gancho numa thread com laço de mensagens próprio. Devolve se instalou.
pub fn instalar(visao: Arc<Mutex<Visao>>, emissor: Sender<Entrada>, relogio: Instant) -> bool {
    if ESTADO.get().is_some() { return true; }
    let _ = ESTADO.set(Estado {
        visao, emissor: Mutex::new(emissor), relogio, apertada: AtomicBool::new(false),
        consumidas: Mutex::new(HashSet::new()), abaixadas: Mutex::new(HashSet::new()),
    });
    let (tx, rx) = std::sync::mpsc::channel::<bool>();
    let _ = thread::Builder::new().name("ditase.gancho".into()).spawn(move || unsafe {
        let modulo = GetModuleHandleW(None).ok();
        let gancho = SetWindowsHookExW(WH_KEYBOARD_LL, Some(ao_teclar), modulo.map(|m| m.into()), 0);
        let _ = tx.send(gancho.is_ok());
        if gancho.is_err() { return; }
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    });
    let ok = rx.recv().unwrap_or(false);
    crate::caminhos::nota(if ok { "atalho: gancho instalado" } else { "atalho: gancho NÃO instalado" });
    ok
}

unsafe extern "system" fn ao_teclar(codigo: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if codigo == HC_ACTION as i32 {
        let tecla = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
        if tecla.flags.0 & LLKHF_INJECTED.0 == 0 && decidir(tecla.vkCode, wparam.0 as u32) {
            return LRESULT(1);
        }
    }
    CallNextHookEx(None, codigo, wparam, lparam)
}

/// Decide na hora se a tecla é nossa. Devolve `true` para engolir.
fn decidir(vk: u32, mensagem: u32) -> bool {
    let Some(e) = ESTADO.get() else { return false };
    let v = *e.visao.lock().unwrap();
    let enviar = |x: Entrada| { let _ = e.emissor.lock().unwrap().send(x); };
    let agora = e.relogio.elapsed().as_secs_f64();
    let desceu = mensagem == WM_KEYDOWN || mensagem == WM_SYSKEYDOWN;
    let subiu = mensagem == WM_KEYUP || mensagem == WM_SYSKEYUP;

    if vk == v.vk {
        // A tecla do atalho passa adiante: Ctrl/Shift/Alt sozinhos não fazem nada nos apps.
        if desceu && !e.apertada.swap(true, Ordering::SeqCst) { enviar(Entrada::Gatilho(true, agora)); }
        if subiu && e.apertada.swap(false, Ordering::SeqCst) { enviar(Entrada::Gatilho(false, agora)); }
        return false;
    }
    if subiu {
        e.abaixadas.lock().unwrap().remove(&vk);
        return e.consumidas.lock().unwrap().remove(&vk);
    }
    if !desceu { return false; }
    if e.consumidas.lock().unwrap().contains(&vk) { return true; }
    let repeticao = !e.abaixadas.lock().unwrap().insert(vk);

    let ctrl = descida(VK_CONTROL);
    let alt = descida(VK_MENU);
    let shift = descida(VK_SHIFT);
    let win = descida(VK_LWIN) || descida(VK_RWIN);
    // Modificadores aceitos junto do Esc: nenhum, ou só o próprio atalho.
    let so_o_atalho = {
        let proprio = |grupo: i32| match grupo { VK_CONTROL => v.vk == 0xA3 || v.vk == 0xA5, VK_SHIFT => v.vk == 0xA1, VK_MENU => v.vk == 0xA5, _ => false };
        (!ctrl || proprio(VK_CONTROL)) && (!alt || proprio(VK_MENU)) && (!shift || proprio(VK_SHIFT)) && !win
    };

    if vk == VK_ESCAPE && v.ativo && so_o_atalho {
        e.consumidas.lock().unwrap().insert(vk);
        if !repeticao { enviar(Entrada::Esc); }
        return true;
    }
    if vk == VK_SPACE && v.segurando {
        e.consumidas.lock().unwrap().insert(vk);
        if !repeticao { enviar(Entrada::Espaco); }
        return true;
    }
    if vk == VK_V && ctrl && alt && !shift && !win {
        e.consumidas.lock().unwrap().insert(vk);
        if !repeticao { enviar(Entrada::ColarUltimo); }
        return true;
    }
    // Modificadores (inclusive o Ctrl falso que o AltGr gera) não contam como "outra tecla".
    if v.segurando && !repeticao && !modificador(vk) { enviar(Entrada::OutraTecla(agora)); }
    false
}
