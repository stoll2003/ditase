//! Colagem (porta de `Insersor.swift`): guarda a área de transferência, põe o texto, aperta
//! Ctrl+V com `SendInput` e devolve o conteúdo anterior depois que o app de destino leu.
//!
//! - App aberto como administrador não recebe teclas de um app comum (UIPI): o texto fica
//!   copiado e a pílula avisa.
//! - O item temporário vai marcado para o histórico do Windows (Win+V) e a sincronização na
//!   nuvem não guardarem.
//! - Formatos guardados: todos os baseados em memória (texto, RTF, HTML, imagem DIB, arquivos).
//!   Formatos de objeto GDI (bitmap, metarquivo, paleta) não voltam; o Windows recria o bitmap a
//!   partir do DIB.
use std::{
    sync::{atomic::{AtomicIsize, Ordering}, Mutex},
    thread,
    time::Duration,
};

use windows::{
    core::{w, PCWSTR},
    Win32::{
        Foundation::{CloseHandle, HANDLE, HGLOBAL, HWND},
        Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY},
        System::{
            DataExchange::{CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData, GetClipboardSequenceNumber, OpenClipboard, RegisterClipboardFormatW, SetClipboardData},
            Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE},
            Threading::{GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION},
        },
        UI::{
            Input::KeyboardAndMouse::{SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, VIRTUAL_KEY, VK_CONTROL, VK_V},
            WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId},
        },
    },
};

use super::Insercao;

const CF_UNICODETEXT: u32 = 13;
/// Formatos que não são memória global: não dá para copiar os bytes.
const GDI: &[u32] = &[2, 3, 9, 14, 0x80, 0x82, 0x83, 0x8E];

static DONO: AtomicIsize = AtomicIsize::new(0);
/// Reserva do conteúdo do usuário e o número de sequência do nosso texto (duas colagens seguidas).
static RESERVA: Mutex<Option<(Vec<(u32, Vec<u8>)>, u32)>> = Mutex::new(None);

/// A área de transferência precisa de uma janela dona: a principal do app.
pub fn definir_dono(hwnd: isize) { DONO.store(hwnd, Ordering::SeqCst); }

struct Aberta;
impl Drop for Aberta { fn drop(&mut self) { unsafe { let _ = CloseClipboard(); } } }

fn abrir() -> Result<Aberta, String> {
    let dono = HWND(DONO.load(Ordering::SeqCst) as *mut _);
    for _ in 0..20 {
        if unsafe { OpenClipboard(Some(dono)) }.is_ok() { return Ok(Aberta); }
        thread::sleep(Duration::from_millis(15));
    }
    Err("área de transferência ocupada por outro programa".into())
}

fn guardar() -> Vec<(u32, Vec<u8>)> {
    let mut itens = Vec::new();
    let Ok(_a) = abrir() else { return itens };
    let mut formato = 0u32;
    loop {
        formato = unsafe { EnumClipboardFormats(formato) };
        if formato == 0 { break; }
        if GDI.contains(&formato) || (0x300..=0x3FF).contains(&formato) { continue; }
        unsafe {
            let Ok(h) = GetClipboardData(formato) else { continue };
            let g = HGLOBAL(h.0);
            let tamanho = GlobalSize(g);
            let p = GlobalLock(g) as *const u8;
            if p.is_null() || tamanho == 0 { continue; }
            itens.push((formato, std::slice::from_raw_parts(p, tamanho).to_vec()));
            let _ = GlobalUnlock(g);
        }
    }
    itens
}

fn por(formato: u32, bytes: &[u8]) {
    unsafe {
        let Ok(g) = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1)) else { return };
        let p = GlobalLock(g) as *mut u8;
        if p.is_null() { return; }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
        let _ = GlobalUnlock(g);
        let _ = SetClipboardData(formato, Some(HANDLE(g.0)));
    }
}

fn texto_utf16(texto: &str) -> Vec<u8> {
    texto.encode_utf16().chain(std::iter::once(0)).flat_map(u16::to_le_bytes).collect()
}

fn escrever(texto: &str, transitorio: bool) -> Result<u32, String> {
    let _a = abrir()?;
    unsafe { EmptyClipboard().map_err(|e| e.to_string())?; }
    por(CF_UNICODETEXT, &texto_utf16(texto));
    if transitorio {
        unsafe {
            por(RegisterClipboardFormatW(w!("ExcludeClipboardContentFromMonitorProcessing")), &[]);
            por(RegisterClipboardFormatW(w!("CanIncludeInClipboardHistory")), &0u32.to_le_bytes());
            por(RegisterClipboardFormatW(w!("CanUploadToCloudClipboard")), &0u32.to_le_bytes());
        }
    }
    let _ = PCWSTR::null();
    Ok(unsafe { GetClipboardSequenceNumber() })
}

pub fn copiar(texto: &str) -> Result<(), String> { escrever(texto, false).map(|_| ()) }

fn elevado(processo: HANDLE) -> Option<bool> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(processo, TOKEN_QUERY, &mut token).ok()?;
        let mut el = TOKEN_ELEVATION::default();
        let mut tam = 0u32;
        let r = GetTokenInformation(token, TokenElevation, Some(&mut el as *mut _ as *mut _), std::mem::size_of::<TOKEN_ELEVATION>() as u32, &mut tam);
        let _ = CloseHandle(token);
        r.ok()?;
        Some(el.TokenIsElevated != 0)
    }
}

/// O app em primeiro plano roda como administrador e nós não: o Windows descarta nosso Ctrl+V.
fn destino_bloqueado() -> bool {
    unsafe {
        let janela = GetForegroundWindow();
        let mut pid = 0u32;
        GetWindowThreadProcessId(janela, Some(&mut pid));
        if pid == 0 { return false; }
        let Ok(p) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return false };
        let alvo = elevado(p);
        let _ = CloseHandle(p);
        alvo == Some(true) && elevado(GetCurrentProcess()) != Some(true)
    }
}

fn tecla(vk: VIRTUAL_KEY, solta: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: 0, dwFlags: if solta { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) }, time: 0, dwExtraInfo: 0 } },
    }
}

pub fn inserir(texto: &str) -> Insercao {
    if destino_bloqueado() {
        let _ = copiar(texto);
        crate::caminhos::nota("colagem: app de destino como administrador; texto só copiado");
        return Insercao::SoCopiado;
    }
    // Duas colagens seguidas: a área ainda tem o nosso texto anterior; vale a reserva da primeira.
    let guardado = {
        let reserva = RESERVA.lock().unwrap().take();
        match reserva {
            Some((itens, nossa)) if unsafe { GetClipboardSequenceNumber() } == nossa => itens,
            _ => guardar(),
        }
    };
    let Ok(sequencia) = escrever(texto, true) else {
        crate::caminhos::nota("colagem: área de transferência ocupada");
        return Insercao::SoCopiado;
    };
    *RESERVA.lock().unwrap() = Some((guardado, sequencia));
    let entradas = [tecla(VK_CONTROL, false), tecla(VK_V, false), tecla(VK_V, true), tecla(VK_CONTROL, true)];
    let enviadas = unsafe { SendInput(&entradas, std::mem::size_of::<INPUT>() as i32) };
    if enviadas != entradas.len() as u32 {
        crate::caminhos::nota("colagem: SendInput bloqueado");
        let _ = copiar(texto);
        RESERVA.lock().unwrap().take();
        return Insercao::SoCopiado;
    }
    // Texto longo demora mais para o app de destino ler.
    let espera = (0.7 + texto.chars().count() as f64 / 20_000.0).min(3.0);
    thread::spawn(move || {
        thread::sleep(Duration::from_secs_f64(espera));
        let mut reserva = RESERVA.lock().unwrap();
        let Some((itens, nossa)) = reserva.as_ref() else { return };
        if *nossa != sequencia { return; } // houve outra colagem depois: ela restaura
        let itens = itens.clone();
        *reserva = None;
        drop(reserva);
        // O usuário copiou outra coisa nesse meio tempo: fica a dele.
        if unsafe { GetClipboardSequenceNumber() } != sequencia { return; }
        if let Ok(_a) = abrir() {
            unsafe { let _ = EmptyClipboard(); }
            for (formato, bytes) in &itens { por(*formato, bytes); }
        }
    });
    Insercao::Colado
}
