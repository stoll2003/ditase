use crate::FonteDeAudio;
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct IdBloco {
    pub documento: String,
    pub fonte: FonteDeAudio,
    pub geracao: u64,
    pub seq: u64,
}
impl IdBloco {
    pub fn texto(&self) -> String {
        format!(
            "{}|{}|{}|{}",
            self.documento,
            self.fonte.nome(),
            self.geracao,
            self.seq
        )
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EstadoBloco {
    Capturado,
    Transcrito,
    SemFala,
    Pendente,
    Lacuna,
    Descartado,
}
impl EstadoBloco {
    pub fn nome(&self) -> &'static str {
        match self {
            Self::Capturado => "capturado",
            Self::Transcrito => "transcrito",
            Self::SemFala => "semFala",
            Self::Pendente => "pendente",
            Self::Lacuna => "lacuna",
            Self::Descartado => "descartado",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlocoRegistrado {
    pub id: IdBloco,
    pub inicio: usize,
    pub quantidade: usize,
    pub arquivo: Option<String>,
    pub soma: Option<String>,
    pub estado: EstadoBloco,
    pub tentativas: u32,
    pub erro: Option<String>,
    pub audio_liberado: bool,
}
pub(crate) fn fonte(s: &str) -> Option<FonteDeAudio> {
    match s {
        "mic" => Some(FonteDeAudio::Mic),
        "mac" => Some(FonteDeAudio::Mac),
        "arquivo" => Some(FonteDeAudio::Arquivo),
        _ => None,
    }
}
/// Prefixo do WAV temporário da identificação de falantes, dentro da pasta da reunião.
pub const PREFIXO_WAV: &str = "falantes-";
/// SHA-256 em hexadecimal dos bytes de um bloco (nome do arquivo e registro).
pub fn soma(dados: &[u8]) -> String {
    format!("{:x}", Sha256::digest(dados))
}
pub(crate) fn uuid(s: &str) -> bool {
    s.len() == 36
        && s.bytes().enumerate().all(|(i, c)| {
            if matches!(i, 8 | 13 | 18 | 23) {
                c == b'-'
            } else {
                c.is_ascii_hexdigit()
            }
        })
}
fn invalido() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "Referência de áudio inválida")
}
fn metadados(p: &Path) -> io::Result<fs::Metadata> {
    let m = fs::symlink_metadata(p)?;
    if m.file_type().is_symlink() {
        return Err(invalido());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if m.file_attributes() & 0x400 != 0 {
            return Err(invalido());
        }
    }
    Ok(m)
}
fn pasta_real(p: &Path) -> io::Result<()> {
    if !metadados(p)?.is_dir() {
        return Err(invalido());
    }
    Ok(())
}
fn sincronizar(p: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        File::open(p)?.sync_all()?
    }
    #[cfg(not(unix))]
    let _ = p;
    Ok(())
}
fn criar_pasta(p: &Path) -> io::Result<()> {
    let mut b = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        b.mode(0o700);
    }
    match b.create(p) {
        Ok(()) => {
            if let Some(parent) = p.parent() {
                sincronizar(parent)?
            }
        }
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }
    pasta_real(p)
}
/// Gravação de um bloco no destino final. A padrão é `DiscoDeBlocos::escrita_duravel`;
/// os testes trocam por uma escrita lenta ou com falha.
pub type Escrita = std::sync::Arc<dyn Fn(&[u8], &Path) -> io::Result<()> + Send + Sync>;
#[derive(Clone)]
pub struct DiscoDeBlocos {
    pub raiz: PathBuf,
    escrita: Escrita,
}
impl DiscoDeBlocos {
    pub fn new(raiz: PathBuf) -> Self {
        Self {
            raiz,
            escrita: std::sync::Arc::new(Self::escrita_duravel),
        }
    }
    /// Como `new`, com outra função de gravação (Swift `init(raiz:escrita:)`).
    pub fn com_escrita(raiz: PathBuf, escrita: Escrita) -> Self {
        Self { raiz, escrita }
    }
    /// Temporário privado (0600), sincronização do arquivo, renomeação e sincronização da pasta.
    pub fn escrita_duravel(dados: &[u8], destino: &Path) -> io::Result<()> {
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let dir = destino.parent().ok_or_else(invalido)?;
        let nome = destino
            .file_name()
            .ok_or_else(invalido)?
            .to_string_lossy()
            .into_owned();
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let tmp = dir.join(format!("{nome}.{}.{seq}.tmp", std::process::id()));
        let mut o = OpenOptions::new();
        o.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            o.mode(0o600);
        }
        let mut f = o.open(&tmp)?;
        let escrito = f.write_all(dados).and_then(|_| f.sync_all());
        drop(f);
        if let Err(e) = escrito.and_then(|_| fs::rename(&tmp, destino)) {
            let _ = fs::remove_file(tmp);
            return Err(e);
        }
        sincronizar(dir)
    }
    /// O arquivo do bloco existe com o tamanho registrado.
    pub fn existe(&self, b: &BlocoRegistrado) -> bool {
        let Some(a) = b.arquivo.as_deref() else {
            return false;
        };
        if !Self::arquivo_valido(a, &b.id.documento) {
            return false;
        }
        metadados(&self.raiz.join(a))
            .map(|m| m.is_file() && Some(m.len()) == (b.quantidade as u64).checked_mul(4))
            .unwrap_or(false)
    }
    /// Remove sem erro (Swift `remover`). Caminho inválido não é seguido.
    pub fn remover(&self, arquivo: &str) {
        if let Some((doc, _)) = arquivo.split_once('/') {
            let _ = self.apagar(doc, arquivo);
        }
    }
    /// Remove a pasta de uma reunião, só se for pasta real com nome UUID (link nunca é seguido).
    pub fn remover_pasta(&self, documento: &str) {
        if !uuid(documento) {
            return;
        }
        let p = self.raiz.join(documento);
        if metadados(&p).map(|m| m.is_dir()).unwrap_or(false) {
            let _ = fs::remove_dir_all(p);
        }
    }
    /// Junta as amostras de uma fonte e geração em `de..ate`, com zeros onde não há áudio.
    /// Devolve também os blocos cujo arquivo falhou na leitura.
    pub fn faixa(
        &self,
        blocos: &[&BlocoRegistrado],
        de: usize,
        ate: usize,
    ) -> (Vec<f32>, Vec<BlocoRegistrado>) {
        let mut saida = vec![0.; ate.saturating_sub(de)];
        let mut quebrados = vec![];
        for &b in blocos {
            let fim = b.inicio + b.quantidade;
            if fim <= de
                || b.inicio >= ate
                || b.arquivo.is_none()
                || matches!(b.estado, EstadoBloco::Lacuna | EstadoBloco::Descartado)
            {
                continue;
            }
            match self.ler(b) {
                Ok(a) => {
                    let x = de.max(b.inicio);
                    let z = ate.min(fim);
                    saida[x - de..z - de].copy_from_slice(&a[x - b.inicio..z - b.inicio])
                }
                Err(_) => quebrados.push(b.clone()),
            }
        }
        (saida, quebrados)
    }
    pub fn pasta(&self, documento: &str) -> io::Result<PathBuf> {
        if !uuid(documento) {
            return Err(invalido());
        }
        criar_pasta(&self.raiz)?;
        let p = self.raiz.join(documento);
        criar_pasta(&p)?;
        Ok(p)
    }
    pub fn arquivo_valido(arquivo: &str, documento: &str) -> bool {
        let Some((d, n)) = arquivo.split_once('/') else {
            return false;
        };
        uuid(d)
            && d == documento
            && n.len() > 4
            && n.ends_with(".f32")
            && !n.starts_with('.')
            && n.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
    }
    fn caminho(&self, doc: &str, arquivo: &str) -> io::Result<PathBuf> {
        if !Self::arquivo_valido(arquivo, doc) {
            return Err(invalido());
        }
        pasta_real(&self.raiz)?;
        pasta_real(&self.raiz.join(doc))?;
        let p = self.raiz.join(arquivo);
        if !metadados(&p)?.is_file() {
            return Err(invalido());
        }
        Ok(p)
    }
    pub fn apagar(&self, documento: &str, arquivo: &str) -> io::Result<()> {
        match self.caminho(documento, arquivo) {
            Ok(p) => {
                fs::remove_file(&p)?;
                sincronizar(p.parent().unwrap())
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }
    pub fn gravar(
        &self,
        id: IdBloco,
        inicio: usize,
        amostras: &[f32],
    ) -> io::Result<BlocoRegistrado> {
        if amostras.is_empty() {
            return Err(invalido());
        }
        let dados: Vec<_> = amostras.iter().flat_map(|a| a.to_le_bytes()).collect();
        let soma = format!("{:x}", Sha256::digest(&dados));
        let nome = format!(
            "{}_{}_{}_{}_{}_{}.f32",
            id.fonte.nome(),
            id.geracao,
            id.seq,
            inicio,
            amostras.len(),
            soma
        );
        let dir = self.pasta(&id.documento)?;
        let destino = dir.join(&nome);
        let arquivo = Some(format!("{}/{}", id.documento, nome));
        let b = BlocoRegistrado {
            id,
            inicio,
            quantidade: amostras.len(),
            soma: Some(soma),
            arquivo,
            estado: EstadoBloco::Capturado,
            tentativas: 0,
            erro: None,
            audio_liberado: false,
        };
        match metadados(&destino) {
            Ok(_) => {
                self.ler(&b)?;
                return Ok(b);
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        (self.escrita)(&dados, &destino)?;
        Ok(b)
    }
    pub fn ler(&self, b: &BlocoRegistrado) -> io::Result<Vec<f32>> {
        let p = self.caminho(&b.id.documento, b.arquivo.as_deref().ok_or_else(invalido)?)?;
        let dados = fs::read(p)?;
        if Some(dados.len()) != b.quantidade.checked_mul(4)
            || b.soma.as_deref() != Some(&format!("{:x}", Sha256::digest(&dados)))
        {
            return Err(invalido());
        }
        Ok(dados
            .as_chunks::<4>()
            .0
            .iter()
            .map(|s| f32::from_le_bytes(*s))
            .collect())
    }
    pub fn orfaos(
        &self,
        registrados: &HashSet<String>,
    ) -> io::Result<(Vec<BlocoRegistrado>, Vec<String>)> {
        let mut blocos = vec![];
        let mut invalidos = vec![];
        match pasta_real(&self.raiz) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok((blocos, invalidos)),
            r => r?,
        }
        for entrada in fs::read_dir(&self.raiz)? {
            let entrada = entrada?;
            let doc = entrada.file_name().to_string_lossy().into_owned();
            if !uuid(&doc) || pasta_real(&entrada.path()).is_err() {
                continue;
            }
            for a in fs::read_dir(entrada.path())? {
                let a = a?;
                let nome = a.file_name().to_string_lossy().into_owned();
                let rel = format!("{doc}/{nome}");
                if fs::symlink_metadata(a.path())
                    .map(|m| m.file_type().is_symlink())
                    .unwrap_or(false)
                {
                    invalidos.push(rel);
                    continue;
                }
                // Temporário nunca renomeado não é durável; WAV da identificação sobrou de uma saída no meio.
                if nome.ends_with(".tmp")
                    || (nome.starts_with(PREFIXO_WAV) && nome.ends_with(".wav"))
                {
                    let _ = fs::remove_file(a.path());
                    continue;
                }
                if registrados.contains(&rel) {
                    continue;
                }
                let Some(base) = nome.strip_suffix(".f32") else {
                    continue;
                };
                let partes: Vec<_> = base.split('_').collect();
                let parse = || {
                    let [f, g, s, i, q, h] = partes.as_slice() else {
                        return None;
                    };
                    Some(BlocoRegistrado {
                        id: IdBloco {
                            documento: doc.clone(),
                            fonte: fonte(f)?,
                            geracao: g.parse().ok()?,
                            seq: s.parse().ok()?,
                        },
                        inicio: i.parse().ok()?,
                        quantidade: q.parse::<usize>().ok().filter(|&n| n > 0)?,
                        arquivo: Some(rel.clone()),
                        soma: Some(h.to_string()),
                        estado: EstadoBloco::Capturado,
                        tentativas: 0,
                        erro: None,
                        audio_liberado: false,
                    })
                };
                match parse() {
                    Some(b) if self.ler(&b).is_ok() => blocos.push(b),
                    _ => invalidos.push(rel),
                }
            }
        }
        blocos.sort_by_key(|b| {
            (
                b.id.documento.clone(),
                b.id.fonte.nome(),
                b.id.geracao,
                b.inicio,
                b.id.seq,
            )
        });
        invalidos.sort();
        Ok((blocos, invalidos))
    }
}

impl DiscoDeBlocos {
    pub fn escrever_wav(&self, blocos: &[&BlocoRegistrado], destino: &Path) -> io::Result<()> {
        use std::io::{Seek, SeekFrom};
        let mut ordenados = blocos.to_vec();
        ordenados.sort_by_key(|b| b.inicio);
        let total = ordenados
            .iter()
            .map(|b| b.inicio + b.quantidade)
            .max()
            .unwrap_or(0);
        let bytes: u32 = total
            .checked_mul(4)
            .and_then(|n| n.try_into().ok())
            .filter(|&n| n <= u32::MAX - 36)
            .ok_or_else(invalido)?;
        let mut o = OpenOptions::new();
        o.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            o.mode(0o600);
        }
        let mut f = o.open(destino)?;
        let gravar = || -> io::Result<()> {
            f.write_all(b"RIFF")?;
            f.write_all(&(36 + bytes).to_le_bytes())?;
            f.write_all(b"WAVEfmt ")?;
            f.write_all(&16u32.to_le_bytes())?;
            f.write_all(&3u16.to_le_bytes())?;
            f.write_all(&1u16.to_le_bytes())?;
            f.write_all(&16000u32.to_le_bytes())?;
            f.write_all(&64000u32.to_le_bytes())?;
            f.write_all(&4u16.to_le_bytes())?;
            f.write_all(&32u16.to_le_bytes())?;
            f.write_all(b"data")?;
            f.write_all(&bytes.to_le_bytes())?;
            f.set_len(44 + u64::from(bytes))?;
            let mut fim = 0;
            for b in ordenados {
                if b.estado == EstadoBloco::Lacuna
                    || b.estado == EstadoBloco::Descartado
                    || b.inicio + b.quantidade <= fim
                {
                    continue;
                }
                let amostras = self.ler(b)?;
                let inicio = fim.max(b.inicio);
                f.seek(SeekFrom::Start(44 + 4 * inicio as u64))?;
                let dados: Vec<_> = amostras[inicio - b.inicio..]
                    .iter()
                    .flat_map(|a| a.to_le_bytes())
                    .collect();
                f.write_all(&dados)?;
                fim = b.inicio + b.quantidade
            }
            f.sync_all()
        };
        let r = gravar();
        drop(f);
        if r.is_err() {
            let _ = fs::remove_file(destino);
        }
        r
    }
}
