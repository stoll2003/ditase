//! Onde o app guarda as coisas no Windows: tudo em `%LOCALAPPDATA%\Dita.se` (porta de
//! `Caminhos.swift`). Banco, registro e áudio ficam fora do perfil móvel: em rede corporativa o
//! `%APPDATA%` pode ir para um servidor, e o OneDrive não sincroniza `%LOCALAPPDATA%`.
//! Versões anteriores guardavam banco e registro em `%APPDATA%\Dita.se`: [`preparar`] migra uma vez,
//! com o SQLite ainda fechado e junto com os arquivos do WAL.
//! Fora do Windows (testes no Mac) usa a pasta temporária, para nunca tocar nos dados do app do Mac.
//! No Windows não há volta para a pasta temporária: sem pasta do perfil, o app não abre.
use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

const PASTA: &str = "Dita.se";
const BANCO: &str = "ditado.sqlite";
/// Arquivos que mudam de `%APPDATA%` para `%LOCALAPPDATA%`. Os três do SQLite vão juntos, com
/// `ditado.sqlite` por último: a presença dele no destino marca migração completa (M-2).
const MIGRADOS: &[&str] = &[
    "ditado.sqlite-wal",
    "ditado.sqlite-shm",
    "preferencias.json",
    "ditado.log",
    BANCO,
];

static RAIZ: OnceLock<PathBuf> = OnceLock::new();
/// Pasta local já resolvida por `preparar` (caminho real).
static LOCAL: OnceLock<PathBuf> = OnceLock::new();

/// Pasta local do perfil. `DITASE_RAIZ` substitui (testes e diagnóstico).
fn local() -> Option<PathBuf> {
    if let Some(outra) = std::env::var_os("DITASE_RAIZ").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(outra));
    }
    if !cfg!(windows) {
        return Some(std::env::temp_dir().join("ditase-dev"));
    }
    std::env::var_os("LOCALAPPDATA")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .filter(|v| !v.is_empty())
                .map(|p| PathBuf::from(p).join("AppData").join("Local"))
        })
        .map(|p| p.join(PASTA))
}

/// Onde as versões anteriores guardavam banco e registro.
fn antiga() -> Option<PathBuf> {
    if !cfg!(windows) || std::env::var_os("DITASE_RAIZ").is_some() {
        return None;
    }
    std::env::var_os("APPDATA")
        .filter(|v| !v.is_empty())
        .map(|p| PathBuf::from(p).join(PASTA))
}

/// Caminho real, sem links nem junções no meio: o núcleo recusa gravar áudio por caminho com link
/// (proteção contra junções). Uma pasta `AppData` realocada por junção continua funcionando porque
/// o app passa a usar o destino real. No Windows tira o prefixo `\\?\` que `canonicalize` põe.
/// Se `canonicalize` falhar (por exemplo volume montado sem letra), cai no caminho original com
/// nota no log, sem travar (M-6).
pub fn real(p: &Path) -> PathBuf {
    let c = match fs::canonicalize(p) {
        Ok(c) => c,
        Err(e) => {
            nota(format!(
                "caminho: não resolvi o destino real ({}); usando o caminho original",
                e.kind()
            ));
            return p.to_path_buf();
        }
    };
    let texto = c.to_string_lossy();
    match texto.strip_prefix(r"\\?\") {
        Some(resto) if !resto.starts_with("UNC\\") => PathBuf::from(resto),
        _ => c,
    }
}

/// Pasta temporária para testes, no caminho real (no Mac, `/tmp` e `/var` são links).
#[cfg(test)]
pub fn pasta_de_teste() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("ditase-teste")
        .tempdir_in(real(&std::env::temp_dir()))
        .unwrap()
}

/// Chamar uma vez, antes de abrir banco ou registro. Cria a pasta e migra os dados antigos.
/// Erro: não há pasta do perfil utilizável (o app avisa e fecha).
pub fn preparar() -> Result<(), String> {
    let nova = local().ok_or("O Windows não informou a pasta do perfil (LOCALAPPDATA).")?;
    fs::create_dir_all(&nova).map_err(|e| format!("Não consegui criar a pasta do Dita.se: {e}"))?;
    let nova = real(&nova);
    let _ = LOCAL.set(nova.clone());
    let mut raiz = nova.clone();
    if let Some(velha) = antiga().filter(|v| v != &nova) {
        match migrar(&velha, &nova) {
            Ok(n) if n > 0 => {
                let _ = RAIZ.set(nova);
                nota(format!(
                    "dados migrados para a pasta local ({n} arquivo(s))"
                ));
                return Ok(());
            }
            Ok(_) => {}
            // Falhou no meio: nada foi apagado na origem; esta sessão usa a pasta antiga.
            Err(e) => {
                raiz = velha;
                let _ = RAIZ.set(raiz);
                nota(format!(
                    "migração da pasta adiada ({e}); usando a pasta anterior"
                ));
                return Ok(());
            }
        }
    }
    let _ = RAIZ.set(raiz);
    Ok(())
}

/// Move `MIGRADOS` de `velha` para `nova` só se o banco ainda não existir no destino. Copia para
/// nomes temporários, grava em disco, troca os nomes com `ditado.sqlite` por último e só então
/// apaga a origem: uma queda no meio deixa a origem intacta e faz a próxima abertura refazer a
/// migração com o WAL junto. Devolve quantos arquivos mudaram.
pub fn migrar(velha: &Path, nova: &Path) -> io::Result<usize> {
    migrar_com(velha, nova, |a, b| fs::rename(a, b))
}

fn migrar_com(
    velha: &Path,
    nova: &Path,
    mut renomear: impl FnMut(&Path, &Path) -> io::Result<()>,
) -> io::Result<usize> {
    if !velha.join(BANCO).is_file() || nova.join(BANCO).exists() {
        return Ok(0);
    }
    // No Windows, garante que nenhuma instância antiga está com o banco aberto antes de copiar.
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        let _exclusivo = OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0)
            .open(velha.join(BANCO))?;
    }
    let presentes: Vec<&str> = MIGRADOS
        .iter()
        .copied()
        .filter(|n| velha.join(n).is_file())
        .collect();
    fs::create_dir_all(nova)?;
    let temporario = |n: &str| nova.join(format!("{n}.migrando"));
    let mut copiar = || -> io::Result<()> {
        for n in &presentes {
            fs::copy(velha.join(n), temporario(n))?;
            // No Windows, FlushFileBuffers exige o arquivo aberto para escrita.
            fs::OpenOptions::new()
                .write(true)
                .open(temporario(n))?
                .sync_all()?;
        }
        for n in &presentes {
            renomear(&temporario(n), &nova.join(n))?;
        }
        Ok(())
    };
    if let Err(e) = copiar() {
        for n in &presentes {
            let _ = fs::remove_file(temporario(n));
        }
        // Um banco copiado sem o WAL seria pior que nenhum: desfaz o que já foi trocado.
        for n in &presentes {
            if velha.join(n).is_file() {
                let _ = fs::remove_file(nova.join(n));
            }
        }
        return Err(e);
    }
    for n in &presentes {
        let _ = fs::remove_file(velha.join(n));
    }
    let _ = fs::remove_dir(velha);
    Ok(presentes.len())
}

fn pasta(raiz: PathBuf, nome: &str) -> PathBuf {
    let p = raiz.join(nome);
    let _ = fs::create_dir_all(&p);
    p
}

/// Banco, preferências e registro. Antes de [`preparar`] (testes), a pasta local.
pub fn raiz() -> PathBuf {
    let r = RAIZ
        .get()
        .cloned()
        .or_else(local)
        .expect("pasta do perfil indisponível; chame caminhos::preparar na abertura");
    let _ = fs::create_dir_all(&r);
    r
}
fn raiz_local() -> PathBuf {
    LOCAL
        .get()
        .cloned()
        .or_else(local)
        .expect("pasta do perfil indisponível; chame caminhos::preparar na abertura")
}
/// Áudio guardado dos ditados (reserva para retranscrever) e PCM das reuniões.
pub fn audios() -> PathBuf {
    pasta(raiz_local(), "Audio")
}
pub fn audio_de_reuniao() -> PathBuf {
    pasta(raiz_local(), "ReunioesAudio")
}
/// Pasta privada de trabalho (temporários do app, nunca a pasta temporária do sistema).
pub fn trabalho() -> PathBuf {
    pasta(raiz_local(), "Trabalho")
}
/// Modelos de fala e de IA. `DITASE_PASTA_MODELOS` aponta para outro lugar, como no Mac.
pub fn modelos() -> PathBuf {
    if let Ok(outra) = std::env::var("DITASE_PASTA_MODELOS") {
        if !outra.is_empty() {
            let p = PathBuf::from(outra);
            let _ = fs::create_dir_all(&p);
            return p;
        }
    }
    pasta(raiz_local(), "Models")
}
pub fn banco() -> PathBuf {
    raiz().join("ditado.sqlite")
}
pub fn preferencias() -> PathBuf {
    raiz().join("preferencias.json")
}
pub fn registro() -> PathBuf {
    raiz().join("ditado.log")
}

static TRAVA: Mutex<()> = Mutex::new(());

/// Registro simples em arquivo (porta de `Registro.nota`). Nunca recebe texto ditado.
pub fn nota(mensagem: impl AsRef<str>) {
    let _g = TRAVA.lock().unwrap_or_else(|e| e.into_inner());
    let caminho = registro();
    if fs::metadata(&caminho)
        .map(|m| m.len() > 2_000_000)
        .unwrap_or(false)
    {
        let _ = fs::remove_file(&caminho);
    }
    let agora = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&caminho) {
        let _ = writeln!(f, "{:.3} {}", agora, mensagem.as_ref());
    }
}

/// Últimas `n` linhas do registro, para o relatório de teste.
pub fn ultimas_linhas(n: usize) -> Vec<String> {
    let texto = fs::read_to_string(registro()).unwrap_or_default();
    let linhas: Vec<&str> = texto.lines().collect();
    linhas[linhas.len().saturating_sub(n)..]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// Pasta do executável: onde o instalador põe as DLLs do sherpa-onnx e a pasta `llama\`.
pub fn pasta_do_app() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(PathBuf::from))
        .unwrap_or_default()
}

/// `llama\llama-server.exe` ao lado do app (instalado pelo NSIS, ver `tauri.distribuicao.conf.json`).
pub fn llama_server() -> PathBuf {
    pasta_do_app().join("llama").join(if cfg!(windows) {
        "llama-server.exe"
    } else {
        "llama-server"
    })
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn migra_banco_com_wal_e_shm_e_apaga_a_origem() {
        let t = pasta_de_teste();
        let (velha, nova) = (t.path().join("Roaming"), t.path().join("Local"));
        fs::create_dir_all(&velha).unwrap();
        for (n, c) in [
            ("ditado.sqlite", "db"),
            ("ditado.sqlite-wal", "wal"),
            ("ditado.sqlite-shm", "shm"),
            ("preferencias.json", "{}"),
        ] {
            fs::write(velha.join(n), c).unwrap();
        }
        assert_eq!(migrar(&velha, &nova).unwrap(), 4);
        assert_eq!(
            fs::read_to_string(nova.join("ditado.sqlite-wal")).unwrap(),
            "wal"
        );
        assert!(!velha.exists(), "origem deveria ter saído");
        // Segunda abertura: nada a fazer.
        assert_eq!(migrar(&velha, &nova).unwrap(), 0);
    }

    #[test]
    fn com_banco_no_destino_nao_mexe_em_nada() {
        let t = pasta_de_teste();
        let (velha, nova) = (t.path().join("Roaming"), t.path().join("Local"));
        fs::create_dir_all(&velha).unwrap();
        fs::create_dir_all(&nova).unwrap();
        fs::write(velha.join("ditado.sqlite"), "antigo").unwrap();
        fs::write(nova.join("ditado.sqlite"), "novo").unwrap();
        assert_eq!(migrar(&velha, &nova).unwrap(), 0);
        assert_eq!(
            fs::read_to_string(velha.join("ditado.sqlite")).unwrap(),
            "antigo"
        );
        assert_eq!(
            fs::read_to_string(nova.join("ditado.sqlite")).unwrap(),
            "novo"
        );
    }

    #[test]
    fn falha_no_meio_deixa_a_origem_intacta() {
        let t = pasta_de_teste();
        let velha = t.path().join("Roaming");
        fs::create_dir_all(&velha).unwrap();
        fs::write(velha.join("ditado.sqlite"), "db").unwrap();
        fs::write(velha.join("ditado.sqlite-wal"), "wal").unwrap();
        // Destino é um arquivo: não dá para criar a pasta.
        let nova = t.path().join("arquivo");
        fs::write(&nova, "x").unwrap();
        assert!(migrar(&velha, &nova).is_err());
        assert_eq!(
            fs::read_to_string(velha.join("ditado.sqlite-wal")).unwrap(),
            "wal"
        );
    }

    /// M-2: `-wal` e `-shm` são renomeados antes de `ditado.sqlite`. Uma falha entre renomeações
    /// desfaz o destino, mantém a origem e permite refazer tudo na abertura seguinte.
    #[test]
    fn falha_entre_renomeacoes_desfaz_e_permite_refazer_com_wal() {
        let t = pasta_de_teste();
        let (velha, nova) = (t.path().join("Roaming"), t.path().join("Local"));
        fs::create_dir_all(&velha).unwrap();
        for (n, c) in [
            ("ditado.sqlite", "db"),
            ("ditado.sqlite-wal", "wal"),
            ("ditado.sqlite-shm", "shm"),
        ] {
            fs::write(velha.join(n), c).unwrap();
        }
        let mut ordem = Vec::new();
        let r = migrar_com(&velha, &nova, |a, b| {
            let nome = b.file_name().unwrap().to_string_lossy().to_string();
            ordem.push(nome.clone());
            if nome == "ditado.sqlite" {
                return Err(io::Error::other("falha simulada antes do banco"));
            }
            fs::rename(a, b)
        });
        assert!(r.is_err());
        assert_eq!(
            ordem,
            vec!["ditado.sqlite-wal", "ditado.sqlite-shm", "ditado.sqlite"]
        );
        assert!(velha.join("ditado.sqlite").is_file());
        assert!(velha.join("ditado.sqlite-wal").is_file());
        assert!(!nova.join("ditado.sqlite").exists());
        assert!(!nova.join("ditado.sqlite-wal").exists());
        // Próxima abertura refaz tudo.
        assert_eq!(migrar(&velha, &nova).unwrap(), 3);
        assert_eq!(
            fs::read_to_string(nova.join("ditado.sqlite-wal")).unwrap(),
            "wal"
        );
        assert_eq!(
            fs::read_to_string(nova.join("ditado.sqlite")).unwrap(),
            "db"
        );
    }

    /// M-2: queda abrupta sem desfazer (processo morto após renomear `-wal`, antes do banco).
    /// Na abertura seguinte o banco ainda não está no destino, então a migração roda e traz o WAL.
    #[test]
    fn queda_abrupta_depois_do_wal_refaz_migracao_na_proxima_abertura() {
        let t = pasta_de_teste();
        let (velha, nova) = (t.path().join("Roaming"), t.path().join("Local"));
        fs::create_dir_all(&velha).unwrap();
        fs::create_dir_all(&nova).unwrap();
        fs::write(velha.join("ditado.sqlite"), "db").unwrap();
        fs::write(velha.join("ditado.sqlite-wal"), "wal-atual").unwrap();
        // Sobrou um -wal parcial no destino, mas o banco ainda não chegou lá.
        fs::write(nova.join("ditado.sqlite-wal"), "wal-parcial").unwrap();
        assert_eq!(migrar(&velha, &nova).unwrap(), 2);
        assert_eq!(
            fs::read_to_string(nova.join("ditado.sqlite-wal")).unwrap(),
            "wal-atual"
        );
        assert_eq!(
            fs::read_to_string(nova.join("ditado.sqlite")).unwrap(),
            "db"
        );
    }

    /// M-6: se `canonicalize` falhar, `real` cai no caminho original sem travar.
    #[test]
    fn real_cai_no_caminho_original_quando_canonicalize_falha() {
        let t = pasta_de_teste();
        let inexistente = t.path().join("nao-existe-ainda");
        assert_eq!(real(&inexistente), inexistente);
    }
}
