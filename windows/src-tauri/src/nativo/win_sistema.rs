//! Informações do Windows e abertura de pastas e telas dos Ajustes do sistema.
use std::{os::windows::process::CommandExt, process::Command};

use windows::{
    core::{w, PWSTR},
    Win32::{
        Foundation::{CloseHandle, HWND},
        System::Threading::{
            OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
            PROCESS_QUERY_LIMITED_INFORMATION,
        },
        UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId},
    },
};

const SEM_JANELA: u32 = 0x0800_0000;

fn registro(nome: &str) -> String {
    let saida = Command::new("reg")
        .args([
            "query",
            r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion",
            "/v",
            nome,
        ])
        .creation_flags(SEM_JANELA)
        .output();
    let texto = saida
        .map(|s| String::from_utf8_lossy(&s.stdout).to_string())
        .unwrap_or_default();
    texto
        .lines()
        .find(|l| l.trim_start().starts_with(nome))
        .and_then(|l| l.split("REG_").nth(1))
        .and_then(|r| r.split_once(char::is_whitespace))
        .map(|(_, v)| v.trim().to_string())
        .unwrap_or_default()
}

/// Ex.: "Windows 11 Pro 24H2 (build 26100)". O registro ainda diz "Windows 10" no Windows 11: corrigido pelo build.
pub fn versao() -> String {
    let build = registro("CurrentBuild");
    let mut produto = registro("ProductName");
    if build.parse::<u32>().unwrap_or(0) >= 22000 {
        produto = produto.replace("Windows 10", "Windows 11");
    }
    format!(
        "{} {} (build {})",
        produto,
        registro("DisplayVersion"),
        build
    )
}

fn dword_personalize(nome: &str) -> Option<u32> {
    let saida = Command::new("reg")
        .args([
            "query",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
            "/v",
            nome,
        ])
        .creation_flags(SEM_JANELA)
        .output()
        .ok()?;
    let texto = String::from_utf8_lossy(&saida.stdout);
    let val = texto
        .lines()
        .find(|l| l.trim_start().starts_with(nome))?
        .split("REG_DWORD")
        .nth(1)?
        .trim();
    if let Some(hex) = val.strip_prefix("0x").or_else(|| val.strip_prefix("0X")) {
        u32::from_str_radix(hex, 16).ok()
    } else {
        val.parse::<u32>().ok()
    }
}

/// Tema da barra de tarefas: `true` se a barra for clara (`SystemUsesLightTheme == 1`).
pub fn barra_clara() -> bool {
    dword_personalize("SystemUsesLightTheme") == Some(1)
}

/// Tema dos aplicativos: `true` se escuro (`AppsUseLightTheme == 0`).
pub fn tema_app_escuro() -> bool {
    dword_personalize("AppsUseLightTheme") == Some(0)
}

/// Mica só no Windows 11 (build >= 22000) com "Efeitos de transparência" ligado (`EnableTransparency == 1`).
pub fn suporta_mica() -> bool {
    let build = registro("CurrentBuild").parse::<u32>().unwrap_or(0);
    build >= 22000 && dword_personalize("EnableTransparency") == Some(1)
}

/// Programa em primeiro plano: (executável, nome para exibir). Nunca o título da janela,
/// que pode ter nome de documento ou conversa.
pub fn app_em_foco() -> (String, String) {
    unsafe {
        let janela: HWND = GetForegroundWindow();
        let mut pid = 0u32;
        GetWindowThreadProcessId(janela, Some(&mut pid));
        let Ok(p) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return (String::new(), String::new());
        };
        let mut buf = [0u16; 520];
        let mut tam = buf.len() as u32;
        let ok =
            QueryFullProcessImageNameW(p, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut tam)
                .is_ok();
        let _ = CloseHandle(p);
        let _ = w!("");
        if !ok {
            return (String::new(), String::new());
        }
        let caminho = String::from_utf16_lossy(&buf[..tam as usize]);
        let exe = caminho.rsplit('\\').next().unwrap_or("").to_string();
        let nome = exe
            .trim_end_matches(".exe")
            .trim_end_matches(".EXE")
            .to_string();
        (exe, nome)
    }
}

/// Abre pasta, arquivo ou URI (`ms-settings:`) no programa padrão.
pub fn abrir(alvo: &str) -> std::io::Result<()> {
    Command::new("explorer.exe")
        .arg(alvo)
        .creation_flags(SEM_JANELA)
        .spawn()
        .map(|_| ())
}

/// Bytes livres para o usuário no volume da pasta (o baixador recusa sem espaço).
pub fn espaco_livre(pasta: &std::path::Path) -> Option<u64> {
    use windows::{core::HSTRING, Win32::Storage::FileSystem::GetDiskFreeSpaceExW};
    let mut livre = 0u64;
    unsafe {
        GetDiskFreeSpaceExW(
            &HSTRING::from(pasta.as_os_str()),
            Some(&mut livre),
            None,
            None,
        )
    }
    .ok()?;
    Some(livre)
}

/// Caixa de mensagem antes de haver janela (falha na abertura).
pub fn alerta(titulo: &str, texto: &str) {
    use windows::{
        core::HSTRING,
        Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK},
    };
    unsafe {
        MessageBoxW(
            None,
            &HSTRING::from(texto),
            &HSTRING::from(titulo),
            MB_OK | MB_ICONERROR,
        );
    }
}
