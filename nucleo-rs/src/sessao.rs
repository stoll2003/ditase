use crate::{motores::*, *};
use std::{
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc, Arc,
    },
    thread,
    time::Duration,
};
#[derive(Clone, Debug)]
pub struct ConfigSessao {
    pub pausa_de_paragrafo: f64,
    pub palavras_minimas_no_paragrafo: usize,
    pub palavras_de_contexto: usize,
    pub minimo_para_modelo: usize,
    pub espera_base: f64,
    pub espera_por_palavra: f64,
    pub trechos_em_memoria: usize,
    /// Pasta privada, existente e escolhida pela casca. Sem pasta, a fila usa só RAM.
    pub pasta_de_trabalho: Option<PathBuf>,
    pub segmentador: segmentador::Config,
}
impl Default for ConfigSessao {
    fn default() -> Self {
        Self {
            pausa_de_paragrafo: 2.2,
            palavras_minimas_no_paragrafo: 35,
            palavras_de_contexto: 40,
            minimo_para_modelo: 4,
            espera_base: 8.,
            espera_por_palavra: 0.35,
            trechos_em_memoria: 12,
            pasta_de_trabalho: None,
            segmentador: Default::default(),
        }
    }
}
#[derive(Default, Clone, Debug)]
pub struct ResultadoDitado {
    pub texto: String,
    pub texto_bruto: String,
    pub palavras: usize,
    pub trocas: Vec<Troca>,
    pub limpador: String,
    pub recusas: usize,
    pub falhas: usize,
    pub duracao_fala: f64,
}
pub type AoTrecho = Arc<dyn Fn(usize, &str) + Send + Sync>;
struct Item {
    segmento: Segmento,
    arquivo: Option<std::fs::File>,
}
pub struct SessaoDitado {
    segmentador: Segmentador,
    entrada: Option<mpsc::Sender<Item>>,
    trabalho: Option<thread::JoinHandle<ResultadoDitado>>,
    cancelada: Arc<AtomicBool>,
    memoria: Arc<AtomicUsize>,
    contagem: Arc<AtomicUsize>,
    limite: usize,
    pasta_de_trabalho: Option<PathBuf>,
}
fn minuscula(t: &str, d: &AplicadorDicionario) -> String {
    let Some(p) = Palavras::ocorrencias(t).next() else {
        return t.into();
    };
    let mut chars = t.chars();
    let Some(c) = chars.next() else {
        return t.into();
    };
    if c.is_uppercase()
        && !(p.as_str().chars().count() > 1 && p.as_str() == p.as_str().to_uppercase())
        && !d.vocabulario.iter().any(|v| v.starts_with(p.as_str()))
    {
        format!("{}{}", c.to_lowercase(), chars.as_str())
    } else {
        t.into()
    }
}
fn guardar_pcm(pasta: &Path, pcm: &[f32]) -> std::io::Result<std::fs::File> {
    disco_de_blocos::validar_pasta(pasta)?;
    let mut f = tempfile::tempfile_in(pasta)?;
    let bytes: Vec<_> = pcm.iter().flat_map(|a| a.to_le_bytes()).collect();
    f.write_all(&bytes)?;
    f.seek(SeekFrom::Start(0))?;
    Ok(f)
}

impl SessaoDitado {
    pub fn new(
        transcritor: Arc<dyn Transcritor>,
        limpador: Option<Arc<dyn Limpador>>,
        dicionario: AplicadorDicionario,
        config: ConfigSessao,
        ao_trecho: Option<AoTrecho>,
    ) -> Self {
        let (tx, rx) = mpsc::channel::<Item>();
        let cancelada = Arc::new(AtomicBool::new(false));
        let memoria = Arc::new(AtomicUsize::new(0));
        let contagem = Arc::new(AtomicUsize::new(0));
        let (cancel, cont, mem) = (cancelada.clone(), contagem.clone(), memoria.clone());
        let segmentador = Segmentador::new(config.segmentador.clone());
        let limite = config.trechos_em_memoria;
        let pasta_de_trabalho = config.pasta_de_trabalho.clone();
        let trabalho = thread::spawn(move || {
            let mut r = ResultadoDitado {
                limpador: "regras".into(),
                ..Default::default()
            };
            let mut ordem = 0;
            let mut continua = false;
            let mut palavras_paragrafo = 0;
            for Item { segmento, arquivo } in rx {
                if cancel.load(Ordering::Acquire) {
                    break;
                }
                let amostras = if let Some(mut f) = arquivo {
                    let mut bytes = vec![];
                    if f.read_to_end(&mut bytes).is_err() || bytes.len() % 4 != 0 {
                        r.falhas += 1;
                        continue;
                    }
                    bytes
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .map(|b| f32::from_le_bytes(*b))
                        .collect()
                } else {
                    mem.fetch_sub(1, Ordering::AcqRel);
                    segmento.amostras
                };
                let bruto = match transcritor.transcrever(&amostras) {
                    Ok(t) => t.trim().to_owned(),
                    Err(_) => {
                        r.falhas += 1;
                        continue;
                    }
                };
                if bruto.is_empty() || cancel.load(Ordering::Acquire) {
                    continue;
                }
                let mut bruto = if continua {
                    minuscula(&bruto, &dicionario)
                } else {
                    bruto
                };
                if segmento.cortado_no_meio && bruto.ends_with('.') && !bruto.ends_with("..") {
                    bruto.pop();
                }
                let n = Palavras::contar(&bruto);
                let paragrafo = ordem > 0
                    && !continua
                    && segmento.pausa_antes >= config.pausa_de_paragrafo
                    && palavras_paragrafo >= config.palavras_minimas_no_paragrafo;
                if paragrafo {
                    palavras_paragrafo = 0
                }
                palavras_paragrafo += n;
                cont.fetch_add(n, Ordering::Relaxed);
                if let Some(f) = &ao_trecho {
                    f(ordem, &bruto)
                }
                ordem += 1;
                if !r.texto_bruto.is_empty() {
                    r.texto_bruto.push(' ')
                }
                r.texto_bruto.push_str(&bruto);
                let (base, trocas) = dicionario.aplicar(&RegrasFala::filtrar(&bruto));
                r.trocas.extend(trocas);
                let mut parte = RegrasFala::arrumar_espacos(&base);
                let n = Palavras::contar(&parte);
                if let Some(l) = limpador.as_ref().filter(|_| n >= config.minimo_para_modelo) {
                    let pedido = PedidoLimpeza {
                        transcricao: parte.clone(),
                        anterior: Palavras::cauda(&r.texto, config.palavras_de_contexto).into(),
                        vocabulario: dicionario.vocabulario.clone(),
                    };
                    let l = l.clone();
                    let nome = l.nome().to_owned();
                    let (resposta, resultado) = mpsc::channel();
                    thread::spawn(move || {
                        let _ = resposta.send(l.limpar(&pedido));
                    });
                    match resultado.recv_timeout(Duration::from_secs_f64(
                        config.espera_base + config.espera_por_palavra * n as f64,
                    )) {
                        Ok(Ok(t)) if Guarda::avaliar(&parte, &t).is_none() => {
                            parte = RegrasFala::arrumar_espacos(
                                &dicionario.aplicar(&Fidelidade::fundir(&parte, &t)).0,
                            );
                            r.limpador = nome
                        }
                        _ => r.recusas += 1,
                    }
                }
                if continua {
                    parte = minuscula(&parte, &dicionario);
                    r.texto.truncate(r.texto.trim_end_matches(['.', '…']).len())
                }
                if !r.texto.is_empty() {
                    r.texto.push_str(if paragrafo { "\n\n" } else { " " })
                }
                r.texto.push_str(&parte);
                continua = segmento.cortado_no_meio
                    || bruto.ends_with("...")
                    || bruto.ends_with(['…', ',', ';', ':', '-'])
                    || bruto.ends_with(char::is_alphanumeric);
            }
            r.palavras = Palavras::contar(&r.texto);
            r
        });
        Self {
            segmentador,
            entrada: Some(tx),
            trabalho: Some(trabalho),
            cancelada,
            memoria,
            contagem,
            limite,
            pasta_de_trabalho,
        }
    }
    fn enfileirar(&self, mut s: Segmento) -> Result<(), Erro> {
        let tx = self.entrada.as_ref().ok_or("Sessão encerrada")?;
        let arquivo = if self.memoria.load(Ordering::Acquire) >= self.limite {
            self.pasta_de_trabalho
                .as_deref()
                .and_then(|pasta| guardar_pcm(pasta, &s.amostras).ok())
        } else {
            None
        };
        if arquivo.is_some() {
            s.amostras = Vec::new();
        } else {
            // Se o disco falhar, preserve o trecho em RAM para não descartar fala.
            self.memoria.fetch_add(1, Ordering::AcqRel);
        }
        tx.send(Item {
            segmento: s,
            arquivo,
        })
        .map_err(|_| "Transcrição interrompida".into())
    }
    pub fn alimentar(&mut self, pcm: &[f32]) -> Result<(), Erro> {
        if !self.cancelada.load(Ordering::Acquire) {
            for s in self.segmentador.alimentar(pcm) {
                self.enfileirar(s)?
            }
        }
        Ok(())
    }
    pub fn palavras_ate_agora(&self) -> usize {
        self.contagem.load(Ordering::Relaxed)
    }
    pub fn medidor(&self) -> (f32, f64, f64) {
        (
            self.segmentador.nivel,
            self.segmentador.silencio_corrido(),
            self.segmentador.fala_total,
        )
    }
    pub fn cancelar(&mut self) {
        self.cancelada.store(true, Ordering::Release);
        self.entrada.take();
    }
    pub fn finalizar(mut self) -> Result<ResultadoDitado, Erro> {
        if !self.cancelada.load(Ordering::Acquire) {
            if let Some(s) = self.segmentador.finalizar() {
                self.enfileirar(s)?
            }
        }
        self.entrada.take();
        let mut r = self
            .trabalho
            .take()
            .ok_or("Sessão encerrada")?
            .join()
            .map_err(|_| "Falha no trabalhador do ditado")?;
        r.duracao_fala = self.segmentador.fala_total;
        Ok(r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fila_saturada_usa_arquivo_anonimo_ou_preserva_pcm_na_memoria() {
        let tmp = tempfile::tempdir().unwrap();
        let pasta = tmp.path().canonicalize().unwrap();
        let (tx, rx) = mpsc::channel();
        let mut sessao = SessaoDitado {
            segmentador: Segmentador::default(),
            entrada: Some(tx),
            trabalho: None,
            cancelada: Arc::new(AtomicBool::new(false)),
            memoria: Arc::new(AtomicUsize::new(0)),
            contagem: Arc::new(AtomicUsize::new(0)),
            limite: 0,
            pasta_de_trabalho: Some(pasta.clone()),
        };
        let segmento = Segmento {
            inicio: 0,
            amostras: vec![0.1, -0.2, 0.],
            pausa_antes: 0.,
            cortado_no_meio: false,
        };
        sessao.enfileirar(segmento.clone()).unwrap();
        let Item {
            segmento: armazenado,
            arquivo,
        } = rx.recv().unwrap();
        assert!(armazenado.amostras.is_empty());
        let mut bytes = vec![];
        arquivo.unwrap().read_to_end(&mut bytes).unwrap();
        assert_eq!(
            bytes,
            segmento
                .amostras
                .iter()
                .flat_map(|a| a.to_le_bytes())
                .collect::<Vec<_>>()
        );
        assert_eq!(pasta.read_dir().unwrap().count(), 0);

        for pasta in [None, Some(pasta.join("inexistente"))] {
            sessao.pasta_de_trabalho = pasta;
            sessao.enfileirar(segmento.clone()).unwrap();
            let Item {
                segmento: armazenado,
                arquivo,
            } = rx.recv().unwrap();
            assert!(arquivo.is_none());
            assert_eq!(armazenado.amostras, segmento.amostras);
        }
    }
    #[test]
    fn temporario_some_ao_encerrar_processo_sem_fechar_o_handle() {
        const VAR_PASTA: &str = "DITASE_TESTE_FILA_PASTA";
        if let Some(pasta) = std::env::var_os(VAR_PASTA) {
            let arquivo = guardar_pcm(Path::new(&pasta), &[0.1]).unwrap();
            assert_eq!(arquivo.metadata().unwrap().len(), 4);
            // Simula término do processo sem executar o Drop do arquivo.
            std::process::exit(43);
        }

        let tmp = tempfile::tempdir().unwrap();
        let pasta = tmp.path().canonicalize().unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "sessao::tests::temporario_some_ao_encerrar_processo_sem_fechar_o_handle",
            ])
            .env(VAR_PASTA, &pasta)
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(43));
        assert_eq!(pasta.read_dir().unwrap().count(), 0);
    }
}
