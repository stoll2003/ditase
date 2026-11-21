use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

#[derive(Clone, Debug)]
pub struct ModeloRemoto {
    pub arquivo: String,
    pub url: String,
    pub sha256: String,
    pub tamanho: u64,
}
impl ModeloRemoto {
    pub fn qwen_padrao() -> Self {
        Self {
            arquivo: "Qwen3.5-4B-Q4_K_M.gguf".into(),
            url: "https://huggingface.co/unsloth/Qwen3.5-4B-GGUF/resolve/b0bd07786f94b700012140fd9263b2de83d4f97b/Qwen3.5-4B-Q4_K_M.gguf".into(),
            sha256: "00fe7986ff5f6b463e62455821146049db6f9313603938a70800d1fb69ef11a4".into(),
            tamanho: 2_740_937_888,
        }
    }
}
pub trait TransporteDeDownload {
    fn abrir(&self, url: &str, a_partir_de: u64) -> io::Result<(u16, Box<dyn Read>)>;
}
#[derive(Debug, PartialEq, Eq)]
pub enum Etapa {
    Baixando { feito: u64, total: u64 },
    Conferindo,
}
pub struct BaixadorDeModelo {
    pub pasta: PathBuf,
    pub modelo: ModeloRemoto,
}
impl BaixadorDeModelo {
    pub fn new(pasta: PathBuf, modelo: ModeloRemoto) -> io::Result<Self> {
        if Path::new(&modelo.arquivo)
            .file_name()
            .and_then(|n| n.to_str())
            != Some(&modelo.arquivo)
            || modelo.arquivo.contains(['/', ':', '\\'])
        {
            return Err(io::Error::other("Nome de modelo inválido"));
        }
        Ok(Self { pasta, modelo })
    }
    pub fn destino(&self) -> PathBuf {
        self.pasta.join(&self.modelo.arquivo)
    }
    pub fn parcial(&self) -> PathBuf {
        self.pasta.join(format!("{}.part", self.modelo.arquivo))
    }
    pub fn marcador(&self) -> PathBuf {
        self.pasta
            .join(format!("{}.conferido", self.modelo.arquivo))
    }
    fn assinatura(&self) -> io::Result<String> {
        let m = fs::metadata(self.destino())?;
        if m.len() != self.modelo.tamanho {
            return Err(io::Error::other("Tamanho incorreto"));
        }
        Ok(format!(
            "{} {} {}",
            self.modelo.sha256,
            m.len(),
            m.modified()?
                .duration_since(UNIX_EPOCH)
                .map_err(io::Error::other)?
                .as_nanos()
        ))
    }
    pub fn instalado(&self) -> bool {
        matches!((self.assinatura(),fs::read_to_string(self.marcador())),(Ok(a),Ok(b)) if a==b.trim())
    }
    pub fn precisa_conferir(&self) -> bool {
        self.assinatura().is_ok() && !self.instalado()
    }
    pub fn ja_baixado(&self) -> u64 {
        fs::metadata(self.parcial()).map_or(0, |m| m.len().min(self.modelo.tamanho))
    }
    fn conferir(&self, p: &Path, cancelado: &impl Fn() -> bool) -> io::Result<()> {
        let mut f = File::open(p)?;
        let mut h = Sha256::new();
        let mut buf = vec![0; 1 << 20];
        loop {
            cancelar(cancelado)?;
            let n = f.read(&mut buf)?;
            if n == 0 {
                break;
            }
            h.update(&buf[..n])
        }
        if format!("{:x}", h.finalize()).eq_ignore_ascii_case(&self.modelo.sha256) {
            Ok(())
        } else {
            drop(f);
            fs::remove_file(p)?;
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "O SHA-256 do modelo não confere",
            ))
        }
    }
    fn marcar(&self) -> io::Result<()> {
        fs::write(self.marcador(), self.assinatura()?).map_err(|e| {
            io::Error::new(
                e.kind(),
                format!("Falha ao gravar o registro da conferência: {e}"),
            )
        })
    }
    pub fn conferir_existente(&self) -> io::Result<()> {
        self.conferir(&self.destino(), &|| false)?;
        self.marcar()
    }
    pub fn baixar(
        &self,
        transporte: &dyn TransporteDeDownload,
        livre: impl Fn(&Path) -> Option<u64>,
        cancelado: impl Fn() -> bool,
        mut progresso: impl FnMut(Etapa),
    ) -> io::Result<PathBuf> {
        fs::create_dir_all(&self.pasta)?;
        if self.instalado() {
            return Ok(self.destino());
        }
        if self.precisa_conferir() {
            progresso(Etapa::Conferindo);
            self.conferir(&self.destino(), &cancelado)?;
            self.marcar()?;
            return Ok(self.destino());
        }
        let parcial = self.parcial();
        let total = self.modelo.tamanho;
        let mut feito = match fs::metadata(&parcial) {
            Ok(m) => m.len(),
            Err(e) if e.kind() == io::ErrorKind::NotFound => 0,
            Err(e) => return Err(e),
        };
        if feito > total {
            fs::remove_file(&parcial)?;
            feito = 0
        }
        if feito < total {
            cancelar(&cancelado)?;
            if livre(&self.pasta).is_some_and(|n| n < (total - feito).saturating_add(64 << 20)) {
                return Err(io::Error::new(
                    io::ErrorKind::StorageFull,
                    "Falta espaço no disco",
                ));
            }
            let (status, mut corpo) = transporte.abrir(&self.modelo.url, feito)?;
            match status {
                206 if feito > 0 => {}
                200 => feito = 0,
                _ => return Err(io::Error::other(format!("Erro HTTP {status}"))),
            }
            let mut f = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(feito == 0)
                .append(feito != 0)
                .open(&parcial)?;
            progresso(Etapa::Baixando { feito, total });
            let mut buf = vec![0; 1 << 20];
            loop {
                cancelar(&cancelado)?;
                let n = corpo.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                if n as u64 > total - feito {
                    return Err(io::Error::other("O servidor enviou dados em excesso"));
                }
                f.write_all(&buf[..n])?;
                feito += n as u64;
                progresso(Etapa::Baixando { feito, total })
            }
            f.sync_all()?;
            if feito != total {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "A conexão caiu antes do fim",
                ));
            }
        }
        cancelar(&cancelado)?;
        progresso(Etapa::Conferindo);
        self.conferir(&parcial, &cancelado)?;
        if self.destino().exists() {
            fs::remove_file(self.destino())?
        }
        fs::rename(parcial, self.destino())?;
        self.marcar()?;
        Ok(self.destino())
    }
}
fn cancelar(f: &impl Fn() -> bool) -> io::Result<()> {
    if f() {
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Download cancelado",
        ))
    } else {
        Ok(())
    }
}
