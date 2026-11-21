use nucleo::*;
use sha2::{Digest, Sha256};
use std::{
    cell::{Cell, RefCell},
    fs,
    io::{self, Cursor, Read},
};
struct Transporte {
    dados: Vec<u8>,
    pedidos: RefCell<Vec<u64>>,
    corte: Cell<Option<usize>>,
    ignorar: bool,
    status: u16,
}
impl TransporteDeDownload for Transporte {
    fn abrir(&self, _: &str, inicio: u64) -> io::Result<(u16, Box<dyn Read>)> {
        self.pedidos.borrow_mut().push(inicio);
        if self.status == 0 {
            return Err(io::ErrorKind::NetworkUnreachable.into());
        }
        let de = if self.ignorar { 0 } else { inicio as usize };
        let fim = self.corte.get().unwrap_or(self.dados.len());
        Ok((
            if self.status == 200 && de > 0 {
                206
            } else {
                self.status
            },
            Box::new(Cursor::new(self.dados[de..fim].to_vec())),
        ))
    }
}
fn ambiente() -> (tempfile::TempDir, BaixadorDeModelo, Transporte) {
    let pasta = tempfile::tempdir().unwrap();
    let dados: Vec<_> = (0..(2 << 20) + 123).map(|n| (n % 251) as u8).collect();
    let modelo = ModeloRemoto {
        arquivo: "teste.gguf".into(),
        url: "https://exemplo.invalid".into(),
        sha256: format!("{:x}", Sha256::digest(&dados)),
        tamanho: dados.len() as u64,
    };
    let b = BaixadorDeModelo::new(pasta.path().to_path_buf(), modelo).unwrap();
    (
        pasta,
        b,
        Transporte {
            dados,
            pedidos: Default::default(),
            corte: Cell::new(None),
            ignorar: false,
            status: 200,
        },
    )
}
fn baixar(b: &BaixadorDeModelo, t: &Transporte) -> io::Result<()> {
    b.baixar(t, |_| None, || false, |_| {}).map(drop)
}
#[test]
fn manifesto() {
    let m = ModeloRemoto::qwen_padrao();
    assert_eq!(m.sha256.len(), 64);
    assert!(m.url.contains("b0bd07786f94b700012140fd9263b2de83d4f97b"));
    assert_eq!(m.tamanho, 2_740_937_888)
}
#[test]
fn progresso_hash_nome_final_e_cache() {
    let (_dir, b, t) = ambiente();
    let mut etapas = vec![];
    let p = b
        .baixar(&t, |_| None, || false, |e| etapas.push(e))
        .unwrap();
    assert_eq!(p.file_name().unwrap(), "teste.gguf");
    assert_eq!(fs::read(p).unwrap(), t.dados);
    assert_eq!(etapas.last(), Some(&Etapa::Conferindo));
    let progresso: Vec<_> = etapas
        .iter()
        .filter_map(|e| match e {
            Etapa::Baixando { feito, .. } => Some(*feito),
            _ => None,
        })
        .collect();
    assert!(progresso.is_sorted());
    assert_eq!(progresso.last(), Some(&b.modelo.tamanho));
    assert!(!b.parcial().exists() && b.instalado());
    baixar(&b, &t).unwrap();
    assert_eq!(*t.pedidos.borrow(), [0])
}
#[test]
fn hash_diferente() {
    let (_dir, b, mut t) = ambiente();
    t.dados[900] ^= 1;
    assert_eq!(
        baixar(&b, &t).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    assert!(!b.parcial().exists() && !b.destino().exists())
}
#[test]
fn queda_e_retomada() {
    let (_dir, b, t) = ambiente();
    t.corte.set(Some(1500000));
    assert_eq!(
        baixar(&b, &t).unwrap_err().kind(),
        io::ErrorKind::UnexpectedEof
    );
    assert_eq!(b.ja_baixado(), 1500000);
    t.corte.set(None);
    baixar(&b, &t).unwrap();
    assert_eq!(*t.pedidos.borrow(), [0, 1500000]);
    assert!(b.instalado())
}
#[test]
fn parcial_anterior_e_range_ignorado() {
    for ignorar in [false, true] {
        let (_dir, b, mut t) = ambiente();
        t.ignorar = ignorar;
        fs::write(b.parcial(), &t.dados[..900]).unwrap();
        baixar(&b, &t).unwrap();
        assert_eq!(*t.pedidos.borrow(), [900]);
        assert!(b.instalado())
    }
}
#[test]
fn parcial_maior() {
    let (_dir, b, t) = ambiente();
    fs::write(b.parcial(), vec![0; t.dados.len() + 1]).unwrap();
    baixar(&b, &t).unwrap();
    assert_eq!(*t.pedidos.borrow(), [0])
}
#[test]
fn cancelar_preserva_parte() {
    let (_dir, b, t) = ambiente();
    let parar = Cell::new(false);
    let erro = b
        .baixar(
            &t,
            |_| None,
            || parar.get(),
            |e| {
                if matches!(e,Etapa::Baixando{feito,..} if feito>0) {
                    parar.set(true)
                }
            },
        )
        .unwrap_err();
    assert_eq!(erro.kind(), io::ErrorKind::Interrupted);
    assert!(b.ja_baixado() > 0 && !b.destino().exists());
    baixar(&b, &t).unwrap()
}
#[test]
fn livre_considera_so_falta() {
    let (_dir, b, t) = ambiente();
    let livre = b.modelo.tamanho + (64 << 20) - 100;
    assert_eq!(
        b.baixar(&t, |_| Some(livre), || false, |_| {})
            .unwrap_err()
            .kind(),
        io::ErrorKind::StorageFull
    );
    assert!(t.pedidos.borrow().is_empty());
    fs::write(b.parcial(), &t.dados[..100]).unwrap();
    b.baixar(&t, |_| Some(livre), || false, |_| {}).unwrap();
}
#[test]
fn sem_rede_e_servidor() {
    for status in [0, 503] {
        let (_dir, b, mut t) = ambiente();
        t.status = status;
        assert!(baixar(&b, &t).is_err());
        assert!(!b.parcial().exists() && !b.destino().exists())
    }
}
#[test]
fn existente_invalido() {
    let (_dir, b, t) = ambiente();
    fs::write(b.destino(), vec![0; t.dados.len()]).unwrap();
    assert!(b.precisa_conferir());
    assert!(baixar(&b, &t).is_err());
    assert!(!b.destino().exists() && t.pedidos.borrow().is_empty())
}
#[test]
fn arquivo_trocado_invalida_marcador() {
    let (_dir, b, t) = ambiente();
    baixar(&b, &t).unwrap();
    // No Windows, set_modified exige o arquivo aberto para escrita.
    let f = fs::OpenOptions::new()
        .write(true)
        .open(b.destino())
        .unwrap();
    f.set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(950000000))
        .unwrap();
    assert!(!b.instalado() && b.precisa_conferir());
    b.conferir_existente().unwrap();
    assert!(b.instalado())
}
#[test]
fn falha_de_escrita_do_marcador_preserva_modelo() {
    let (_dir, b, t) = ambiente();
    fs::create_dir(b.marcador()).unwrap();
    let e = baixar(&b, &t).unwrap_err();
    assert!(e.to_string().contains("registro da conferência"));
    assert_eq!(fs::read(b.destino()).unwrap(), t.dados);
    assert!(!b.instalado());
}
