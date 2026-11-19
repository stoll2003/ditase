//! Informações do Windows e abertura de pastas e telas dos Ajustes do sistema.
use std::{os::windows::process::CommandExt, process::Command};

use windows::{
    core::{w, PWSTR},
    Win32::{
        Foundation::{CloseHandle, HWND},
        System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION},
        UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId},
    },
};

const SEM_JANELA: u32 = 0x0800_0000;

fn registro(nome: &str) -> String {
    let saida = Command::new("reg")
        .args(["query", r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion", "/v", nome])
        .creation_flags(SEM_JANELA).output();
    let texto = saida.map(|s| String::from_utf8_lossy(&s.stdout).to_string()).unwrap_or_default();
    texto.lines().find(|l| l.trim_start().starts_with(nome))
        .and_then(|l| l.split("REG_").nth(1)).and_then(|r| r.split_once(char::is_whitespace)).map(|(_, v)| v.trim().to_string())
        .unwrap_or_default()
}

/// Ex.: "Windows 11 Pro 24H2 (build 26100)". O registro ainda diz "Windows 10" no Windows 11: corrigido pelo build.
pub fn versao() -> String {
    let build = registro("CurrentBuild");
    let mut produto = registro("ProductName");
    if build.parse::<u32>().unwrap_or(0) >= 22000 { produto = produto.replace("Windows 10", "Windows 11"); }
    format!("{} {} (build {})", produto, registro("DisplayVersion"), build)
}

/// Programa em primeiro plano: (executável, nome para exibir). Nunca o título da janela,
/// que pode ter nome de documento ou conversa.
pub fn app_em_foco() -> (String, String) {
    unsafe {
        let janela: HWND = GetForegroundWindow();
        let mut pid = 0u32;
        GetWindowThreadProcessId(janela, Some(&mut pid));
        let Ok(p) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return (String::new(), String::new()) };
        let mut buf = [0u16; 520];
        let mut tam = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(p, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut tam).is_ok();
        let _ = CloseHandle(p);
        let _ = w!("");
        if !ok { return (String::new(), String::new()); }
        let caminho = String::from_utf16_lossy(&buf[..tam as usize]);
        let exe = caminho.rsplit('\\').next().unwrap_or("").to_string();
        let nome = exe.trim_end_matches(".exe").trim_end_matches(".EXE").to_string();
        (exe, nome)
    }
}

/// Abre pasta, arquivo ou URI (`ms-settings:`) no programa padrão.
pub fn abrir(alvo: &str) -> std::io::Result<()> {
    Command::new("explorer.exe").arg(alvo).creation_flags(SEM_JANELA).spawn().map(|_| ())
}
