//! Onde o app guarda as coisas no Windows: `%APPDATA%\Dita.se` (porta de `Caminhos.swift`).
//! O áudio fica em `%LOCALAPPDATA%\Dita.se\Audio`: fora do perfil móvel e longe do que o OneDrive
//! costuma sincronizar (Documentos, Área de Trabalho). Fora do Windows (testes no Mac) usa a pasta
//! temporária, para nunca tocar nos dados do app do Mac.
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

const PASTA: &str = "Dita.se";

fn base(variavel: &str) -> PathBuf {
    if let Ok(outra) = std::env::var("DITASE_RAIZ") { if !outra.is_empty() { return PathBuf::from(outra); } }
    if cfg!(windows) {
        if let Some(p) = std::env::var_os(variavel) { return PathBuf::from(p).join(PASTA); }
    }
    std::env::temp_dir().join("ditase-dev")
}

fn pasta(raiz: PathBuf, nome: &str) -> PathBuf {
    let p = raiz.join(nome);
    let _ = fs::create_dir_all(&p);
    p
}

/// Banco, preferências, registro, diários de reunião.
pub fn raiz() -> PathBuf { let r = base("APPDATA"); let _ = fs::create_dir_all(&r); r }
/// Áudio guardado dos ditados (reserva para retranscrever) e PCM das reuniões.
pub fn audios() -> PathBuf { pasta(base("LOCALAPPDATA"), "Audio") }
pub fn audio_de_reuniao() -> PathBuf { pasta(base("LOCALAPPDATA"), "ReunioesAudio") }
/// Modelos de fala e de IA. `DITASE_PASTA_MODELOS` aponta para outro lugar, como no Mac.
pub fn modelos() -> PathBuf {
    if let Ok(outra) = std::env::var("DITASE_PASTA_MODELOS") {
        if !outra.is_empty() { let p = PathBuf::from(outra); let _ = fs::create_dir_all(&p); return p; }
    }
    pasta(base("LOCALAPPDATA"), "Models")
}
pub fn banco() -> PathBuf { raiz().join("ditado.sqlite") }
pub fn preferencias() -> PathBuf { raiz().join("preferencias.json") }
pub fn registro() -> PathBuf { raiz().join("ditado.log") }

static TRAVA: Mutex<()> = Mutex::new(());

/// Registro simples em arquivo (porta de `Registro.nota`). Nunca recebe texto ditado.
pub fn nota(mensagem: impl AsRef<str>) {
    let _g = TRAVA.lock().unwrap_or_else(|e| e.into_inner());
    let caminho = registro();
    if fs::metadata(&caminho).map(|m| m.len() > 2_000_000).unwrap_or(false) { let _ = fs::remove_file(&caminho); }
    let agora = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&caminho) {
        let _ = writeln!(f, "{:.3} {}", agora, mensagem.as_ref());
    }
}

/// Últimas `n` linhas do registro, para o relatório de teste.
pub fn ultimas_linhas(n: usize) -> Vec<String> {
    let texto = fs::read_to_string(registro()).unwrap_or_default();
    let linhas: Vec<&str> = texto.lines().collect();
    linhas[linhas.len().saturating_sub(n)..].iter().map(|s| s.to_string()).collect()
}

/// Pasta do executável: onde o instalador põe as DLLs do sherpa-onnx e a pasta `llama\`.
pub fn pasta_do_app() -> PathBuf {
    std::env::current_exe().ok().and_then(|e| e.parent().map(PathBuf::from)).unwrap_or_default()
}

/// `llama\llama-server.exe` ao lado do app (instalado pelo NSIS, ver `tauri.distribuicao.conf.json`).
pub fn llama_server() -> PathBuf { pasta_do_app().join("llama").join(if cfg!(windows) { "llama-server.exe" } else { "llama-server" }) }
