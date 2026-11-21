//! Arquivos e reuniões (porta de `ServicoLancamento.swift`). A casca captura e decodifica; o
//! núcleo processa, guarda e recupera:
//!
//! - Arquivo: decodifica (Media Foundation/Symphonia) e passa por `ProcessadorDeArquivo` numa fila
//!   própria. A versão nova só substitui a anterior depois de publicada (`ArmazemLancamento::publicar`).
//! - Reunião: microfone e áudio do sistema (loopback) vão para o `ColetorDeReuniao`, que grava cada
//!   bloco em disco antes de qualquer reconhecimento; o `ProcessadorDeReuniao` (outra conexão) lê os
//!   blocos selados, publica os trechos ao vivo e, no fim, faz a passada final de falantes. Queda no
//!   meio deixa o diário em disco: `preparar_abertura` adota e a reunião aparece em `recuperaveis`.
//!
//! Nenhum texto de fala vai para o registro.
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex, MutexGuard,
    },
    thread,
    time::{Duration, Instant},
};

use nucleo::{
    motores::ReconhecedorDeFala, ArmazemLancamento, CampoDocumento, ColetorDeReuniao, Desfecho,
    DiscoDeBlocos, FonteDeAudio, Palavras, ProcessadorDeArquivo, ProcessadorDeReuniao,
    TranscricaoLonga, TrechoNovo,
};
use serde_json::{json, Value};

use crate::{
    audio::Fonte,
    caminhos::nota,
    decodificacao,
    integracao::{Motores, Res},
};

/// Avisos à interface: `("lancamento", None)` invalida o estado; `("ditado:progresso", Some(v))` e afins levam dados.
pub type Avisar = Arc<dyn Fn(&str, Option<Value>) + Send + Sync>;
/// Preparação do texto de cada trecho (regras e dicionário), a mesma do ditado.
pub type Preparar = Arc<dyn Fn(&str) -> String + Send + Sync>;

/// Recebe PCM 16 kHz mono de uma fonte com o tempo (segundos no relógio da reunião) do início do bloco.
pub type AoBloco = Box<dyn FnMut(f64, Vec<f32>, f32) + Send>;

/// Abre fontes de áudio. No app, `cpal` (microfone e loopback); nos testes, um dublê.
pub trait Capturador: Send + Sync {
    /// Devolve um guarda: soltá-lo fecha a fonte.
    fn abrir(
        &self,
        fonte: Fonte,
        relogio: Instant,
        ao_bloco: AoBloco,
    ) -> Result<Box<dyn Send>, String>;
}

/// Tempo de cada bloco de uma fonte, pela contagem de amostras entregues desde o primeiro bloco.
/// O relógio de parede na hora da chamada oscila com o agendamento do sistema; usado direto, um
/// atraso de 60 ms vira lacuna no coletor (tolerância de 960 amostras do núcleo). Só ressincroniza
/// quando a diferença passa de `RESSINCRONIA` (o dispositivo realmente parou ou pulou).
#[derive(Default)]
pub struct RelogioDeFonte {
    inicio: Option<f64>,
    amostras: u64,
}

pub const RESSINCRONIA: f64 = 0.3;

impl RelogioDeFonte {
    /// `agora`: segundos do relógio da reunião no fim do bloco; devolve o início do bloco.
    pub fn tempo(&mut self, agora: f64, n: usize) -> f64 {
        let duracao = n as f64 / crate::audio::TAXA as f64;
        let pelo_relogio = (agora - duracao).max(0.0);
        let inicio = *self.inicio.get_or_insert(pelo_relogio);
        let mut t = inicio + self.amostras as f64 / crate::audio::TAXA as f64;
        // Só ressincroniza para a frente (parada real do dispositivo). Recuar o tempo faria o
        // CursorDeFonte descartar fala nova como repetida (deriva de relógio ou fila acumulada).
        if pelo_relogio - t > RESSINCRONIA {
            self.inicio = Some(pelo_relogio - self.amostras as f64 / crate::audio::TAXA as f64);
            t = pelo_relogio;
        }
        self.amostras += n as u64;
        t
    }
}

/// Capturador real: `audio::Captura`. O tempo do bloco vem do [`RelogioDeFonte`].
pub struct CapturaDoSistema {
    pub microfone: Arc<dyn Fn() -> String + Send + Sync>,
}

impl Capturador for CapturaDoSistema {
    fn abrir(
        &self,
        fonte: Fonte,
        relogio: Instant,
        mut ao_bloco: AoBloco,
    ) -> Result<Box<dyn Send>, String> {
        let nome = if fonte == Fonte::Microfone {
            (self.microfone)()
        } else {
            String::new()
        };
        let mut tempo = RelogioDeFonte::default();
        let c = crate::audio::Captura::iniciar(fonte, &nome, move |bloco, nivel| {
            let t = tempo.tempo(relogio.elapsed().as_secs_f64(), bloco.len());
            ao_bloco(t, bloco, nivel);
        })
        .map_err(|e| e.to_string())?;
        Ok(Box::new(c))
    }
}

#[derive(Clone)]
struct ItemArquivo {
    doc: String,
    nome: String,
    fase: &'static str,
    progresso: Option<f64>,
    motivo: Option<String>,
    cancelado: Arc<AtomicBool>,
}

struct Tarefa {
    doc: String,
    nome: String,
    caminho: PathBuf,
    cancelado: Arc<AtomicBool>,
}

enum Captado {
    Audio(FonteDeAudio, f64, Vec<f32>),
    Pausar(f64),
    Retomar(f64),
    Encerrar(f64),
}

/// Estado ao vivo compartilhado com as threads da reunião.
struct Vivo {
    pausada: AtomicBool,
    nivel: Mutex<f32>,
    ultimo_audio: Mutex<Instant>,
    /// Segundos de pausa acumulados e início da pausa atual (no relógio da reunião).
    pausas: Mutex<(f64, Option<f64>)>,
    relogio: Instant,
}

impl Vivo {
    fn decorrido(&self) -> f64 {
        let agora = self.relogio.elapsed().as_secs_f64();
        let (total, desde) = *self.pausas.lock().unwrap();
        agora - total - desde.map_or(0.0, |d| agora - d)
    }
}

struct Ativa {
    doc: String,
    vivo: Arc<Vivo>,
    capturas: Vec<Box<dyn Send>>,
    envio: mpsc::Sender<Captado>,
    coletor_fim: Arc<AtomicBool>,
    microfone: bool,
}

/// Derivação do microfone da reunião para um ditado (I5). `geracao` muda a cada derivação: uma
/// fonte antiga, solta depois que o ditado seguinte já derivou, não apaga a derivação nova (I-1).
#[derive(Default)]
struct EstadoDerivacao {
    geracao: u64,
    ao_bloco: Option<crate::controlador::AoBlocoDeAudio>,
    /// Chamado quando a reunião solta o microfone com o ditado ainda derivando (I-2).
    ao_acabar: Option<crate::controlador::AoAcabar>,
}
type Derivacao = Arc<Mutex<EstadoDerivacao>>;

/// Fonte do ditado derivada da reunião: soltar ou parar fecha a derivação, se ainda for a dela.
struct FonteDerivada {
    derivacao: Derivacao,
    geracao: u64,
}
impl FonteDerivada {
    fn fechar(&self) {
        let mut d = self.derivacao.lock().unwrap();
        if d.geracao == self.geracao {
            d.ao_bloco = None;
            d.ao_acabar = None;
        }
    }
}
impl crate::controlador::FonteAberta for FonteDerivada {
    fn parar(&mut self) {
        self.fechar();
    }
}
impl Drop for FonteDerivada {
    fn drop(&mut self) {
        self.fechar();
    }
}

pub struct Lancamento {
    derivacao: Derivacao,
    banco: PathBuf,
    disco: DiscoDeBlocos,
    trabalho: PathBuf,
    motores: Arc<dyn Motores>,
    capturador: Arc<dyn Capturador>,
    preparar: Preparar,
    avisar: Avisar,
    leitor: Mutex<ArmazemLancamento>,
    fila: Arc<Mutex<Vec<ItemArquivo>>>,
    caminhos: Mutex<HashMap<String, PathBuf>>,
    envio: Mutex<mpsc::Sender<Tarefa>>,
    reuniao: Mutex<Option<Ativa>>,
    /// Reunião encerrada ou em recuperação que ainda está na passada final ou no resumo.
    finalizando: Arc<Mutex<Option<String>>>,
    resumo_cancelado: Arc<AtomicBool>,
    /// App saindo: a reunião em curso fecha sem resumo e nada novo começa.
    desligando: AtomicBool,
    /// O resumo está sendo gerado agora (a interface mostra "Cancelar resumo" com `etapa: resumindo`).
    resumindo: AtomicBool,
}

fn uuid() -> String {
    use std::hash::{BuildHasher, Hasher};
    use std::sync::atomic::AtomicU64;
    static N: AtomicU64 = AtomicU64::new(0);
    let semente = std::collections::hash_map::RandomState::new();
    let parte = |extra: u64| {
        let mut h = semente.build_hasher();
        h.write_u64(extra);
        h.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
        );
        h.write_u64(N.fetch_add(1, Ordering::Relaxed));
        h.finish()
    };
    let (a, b) = (parte(1), parte(2));
    format!(
        "{:08x}-{:04x}-4{:03x}-{:04x}-{:012x}",
        a >> 32,
        (a >> 16) & 0xffff,
        a & 0xfff,
        0x8000 | (b >> 48 & 0x3fff),
        b & 0xffff_ffff_ffff
    )
}

/// Deixa passar no máximo uma vez por `intervalo`, salvo quando forçado.
pub struct Limitador {
    intervalo: Duration,
    ultimo: Option<Instant>,
}

impl Limitador {
    pub fn new(intervalo: Duration) -> Self {
        Self {
            intervalo,
            ultimo: None,
        }
    }
    pub fn deixar(&mut self, agora: Instant, forcar: bool) -> bool {
        let passa = forcar
            || self
                .ultimo
                .is_none_or(|u| agora.duration_since(u) >= self.intervalo);
        if passa {
            self.ultimo = Some(agora);
        }
        passa
    }
}

/// Fase do arquivo na interface e rótulo da etapa na pílula.
fn rotulo_de_etapa(etapa: &str) -> (&'static str, &'static str) {
    match etapa {
        "transcrevendo" => ("transcrevendo", "Transcrevendo"),
        "diarizando" => ("diarizando", "Separando falantes"),
        _ => ("diarizando", "Preparando o texto"),
    }
}

/// Corpo do evento `ditado:progresso` (contrato da ponte). Sem `progresso` quando a fração é desconhecida.
fn progresso_json(
    id: &str,
    fase: &str,
    etapa: &str,
    rotulo: &str,
    fracao: Option<f64>,
    longo: bool,
    reuniao_ativa: bool,
) -> Value {
    let mut v = json!({"id": id, "fase": fase, "etapa": {"id": etapa, "rotulo": rotulo}, "estimativa": fracao.is_some(), "longo": longo, "reuniaoAtiva": reuniao_ativa});
    if let Some(f) = fracao {
        v["progresso"] = json!(f);
    }
    v
}

/// Marca de "app fechando" no processamento da reunião.
#[derive(Debug)]
struct Desligando;
impl std::fmt::Display for Desligando {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("app fechando")
    }
}
impl std::error::Error for Desligando {}

/// Alterna pausa e gravação. A ordem importa: na pausa, a captura para de enviar antes do
/// `Pausar`; na retomada, o `Retomar` entra no canal antes de a captura voltar a enviar. Assim
/// nenhum bloco de depois da retomada chega ao coletor com a linha do tempo ainda pausada.
fn alternar_pausa(vivo: &Vivo, enviar: impl FnOnce(Captado)) -> bool {
    let t = vivo.relogio.elapsed().as_secs_f64();
    let pausar = !vivo.pausada.load(Ordering::Acquire);
    {
        let mut pausas = vivo.pausas.lock().unwrap();
        if pausar {
            pausas.1 = Some(t);
        } else if let Some(d) = pausas.1.take() {
            pausas.0 += t - d;
        }
    }
    if pausar {
        vivo.pausada.store(true, Ordering::Release);
        enviar(Captado::Pausar(t));
    } else {
        enviar(Captado::Retomar(t));
        vivo.pausada.store(false, Ordering::Release);
    }
    pausar
}

fn trecho_json(t: &TrechoNovo) -> Value {
    json!({"id": t.id, "inicio": t.inicio, "fim": t.fim, "voz": t.voz, "texto": t.texto, "parcial": true})
}

impl Lancamento {
    pub fn new(
        banco: &Path,
        pasta_audio: &Path,
        motores: Arc<dyn Motores>,
        capturador: Arc<dyn Capturador>,
        preparar: Preparar,
        avisar: Avisar,
    ) -> Res<Arc<Self>> {
        let disco = DiscoDeBlocos::new(pasta_audio.to_path_buf());
        let mut leitor = ArmazemLancamento::new(banco)?;
        let r = leitor.preparar_abertura(&disco);
        if r.adotados + r.lacunas > 0 || !r.falhas.is_empty() || !r.invalidos.is_empty() {
            nota(format!("abertura das reuniões: {} bloco(s) adotado(s), {} lacuna(s), {} inválido(s), {} falha(s)", r.adotados, r.lacunas, r.invalidos.len(), r.falhas.len()));
        }
        let trabalho = pasta_audio.join("Trabalho");
        let _ = std::fs::remove_dir_all(&trabalho);
        std::fs::create_dir_all(&trabalho)?;
        let (tx, rx) = mpsc::channel::<Tarefa>();
        let servico = Arc::new(Self {
            banco: banco.to_path_buf(),
            disco,
            trabalho,
            motores,
            capturador,
            preparar,
            avisar,
            derivacao: Arc::new(Mutex::new(EstadoDerivacao::default())),
            leitor: Mutex::new(leitor),
            fila: Arc::new(Mutex::new(Vec::new())),
            caminhos: Mutex::new(HashMap::new()),
            envio: Mutex::new(tx),
            reuniao: Mutex::new(None),
            finalizando: Arc::new(Mutex::new(None)),
            resumo_cancelado: Arc::new(AtomicBool::new(false)),
            desligando: AtomicBool::new(false),
            resumindo: AtomicBool::new(false),
        });
        let fraco = Arc::downgrade(&servico);
        thread::Builder::new()
            .name("ditase.arquivos".into())
            .spawn(move || {
                for t in rx {
                    let Some(s) = fraco.upgrade() else { break };
                    s.processar_arquivo(t);
                }
            })?;
        Ok(servico)
    }

    /// Conexão própria ao banco (processador, coletor e gravações de fim têm cada um a sua).
    fn armazem(&self) -> Result<ArmazemLancamento, nucleo::motores::Erro> {
        ArmazemLancamento::new(&self.banco)
    }
    fn leitor(&self) -> MutexGuard<'_, ArmazemLancamento> {
        self.leitor.lock().unwrap_or_else(|e| e.into_inner())
    }
    fn mudou(&self) {
        (self.avisar)("lancamento", None)
    }

    fn reuniao_atual(&self) -> Option<String> {
        self.reuniao
            .lock()
            .unwrap()
            .as_ref()
            .map(|a| a.doc.clone())
            .or_else(|| self.finalizando.lock().unwrap().clone())
    }

    // MARK: Leitura

    pub fn estado(&self, microfone: bool) -> Res<Value> {
        let atual = self.reuniao_atual();
        let leitor = self.leitor();
        let fila = self.fila.lock().unwrap().clone();
        let mut arquivos: Vec<Value> = Vec::new();
        for d in leitor.lista("arquivo")? {
            if d.oculto == Some(true) {
                continue;
            }
            let mut v = serde_json::to_value(&d)?;
            if let Some(item) = fila.iter().find(|i| i.doc == d.id) {
                v["fase"] = json!(item.fase);
                v["progresso"] = json!(item.progresso);
                if let Some(m) = &item.motivo {
                    v["motivo"] = json!(m);
                }
            }
            arquivos.push(v);
        }
        // Itens que nunca chegaram ao banco (formato não suportado).
        let faltando: Vec<&ItemArquivo> = fila
            .iter()
            .filter(|i| !arquivos.iter().any(|a| a["id"] == json!(i.doc)))
            .collect();
        for item in faltando {
            arquivos.insert(0, json!({"id": item.doc, "nome": item.nome, "criadoEm": crate::integracao::agora(), "vozes": [], "trechos": [], "fase": item.fase, "motivo": item.motivo}));
        }
        let reuniao = match &atual {
            Some(id) => leitor.documento(id)?,
            None => None,
        };
        let mut reuniao = reuniao.map(serde_json::to_value).transpose()?;
        if let (Some(r), Some(f)) = (reuniao.as_mut(), self.finalizando.lock().unwrap().as_ref()) {
            if r["id"] == json!(f) {
                r["fase"] = json!("finalizando");
                r["etapa"] = json!(if self.resumindo.load(Ordering::Acquire) {
                    "resumindo"
                } else {
                    "passadaFinal"
                });
            }
        }
        let anteriores: Vec<TranscricaoLonga> = leitor
            .lista("reuniao")?
            .into_iter()
            .filter(|d| {
                d.oculto != Some(true) && Some(&d.id) != atual.as_ref() && d.fase == "pronta"
            })
            .collect();
        // O núcleo não tem campo próprio para o erro do processamento: a casca guarda em `aviso`.
        let recuperaveis: Vec<Value> = leitor
            .recuperaveis(atual.as_deref())?
            .into_iter()
            .map(|d| {
                let aviso = d.aviso.clone();
                let mut v = serde_json::to_value(d).unwrap_or_default();
                if v.get("erroProcessamento").is_none() {
                    if let Some(a) = aviso {
                        v["erroProcessamento"] = json!(a);
                    }
                }
                v
            })
            .collect();
        let mut v = json!({
            "arquivos": arquivos, "anteriores": anteriores, "recuperaveis": recuperaveis,
            "permissoes": {"audioSistema": "concedida", "microfone": if microfone { "concedida" } else { "negada" }},
            "disponivel": self.motores.fala_disponivel(),
        });
        if let Some(r) = reuniao {
            v["reuniao"] = r;
        }
        Ok(v)
    }

    pub fn documento(&self, id: &str) -> Res<Value> {
        Ok(serde_json::to_value(
            self.leitor()
                .documento(id)?
                .ok_or("Documento inexistente")?,
        )?)
    }

    /// IDs dos arquivos cujo texto contém o termo (sem acento, sem diferença de caixa).
    pub fn procurar_arquivos(&self, termo: &str) -> Res<Value> {
        let alvo = Palavras::normalizadas(termo).join(" ");
        if alvo.is_empty() {
            return Ok(json!([]));
        }
        let leitor = self.leitor();
        let mut ids = Vec::new();
        for d in leitor.lista("arquivo")? {
            if d.oculto == Some(true) {
                continue;
            }
            let Some(completo) = leitor.documento(&d.id)? else {
                continue;
            };
            let texto = completo
                .trechos
                .iter()
                .map(|t| t.texto.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            if Palavras::normalizadas(&format!("{} {}", completo.nome, texto))
                .join(" ")
                .contains(&alvo)
            {
                ids.push(d.id);
            }
        }
        Ok(json!(ids))
    }

    // MARK: Arquivos

    fn enfileirar(&self, doc: String, nome: String, caminho: PathBuf) {
        let cancelado = Arc::new(AtomicBool::new(false));
        self.caminhos
            .lock()
            .unwrap()
            .insert(doc.clone(), caminho.clone());
        let mut fila = self.fila.lock().unwrap();
        fila.retain(|i| i.doc != doc);
        fila.push(ItemArquivo {
            doc: doc.clone(),
            nome: nome.clone(),
            fase: "naFila",
            progresso: None,
            motivo: None,
            cancelado: cancelado.clone(),
        });
        drop(fila);
        let _ = self.envio.lock().unwrap().send(Tarefa {
            doc,
            nome,
            caminho,
            cancelado,
        });
    }

    /// Põe os arquivos na fila, na ordem. Devolve o ID do primeiro.
    pub fn adicionar(&self, arquivos: &[PathBuf]) -> Res<Option<String>> {
        let mut primeiro = None;
        for caminho in arquivos {
            let doc = uuid();
            let nome = caminho
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "Arquivo".into());
            if !decodificacao::suportado(caminho) {
                self.fila.lock().unwrap().push(ItemArquivo {
                    doc: doc.clone(),
                    nome,
                    fase: "formatoNaoSuportado",
                    progresso: None,
                    motivo: Some(
                        "Formato não suportado. Use MP3, M4A, WAV, FLAC, MP4 ou MOV.".into(),
                    ),
                    cancelado: Arc::new(AtomicBool::new(true)),
                });
            } else {
                self.leitor()
                    .criar_documento(&doc, "arquivo", &nome, None, "ativo")?;
                self.enfileirar(doc.clone(), nome, caminho.clone());
            }
            primeiro.get_or_insert(doc);
        }
        self.mudou();
        Ok(primeiro)
    }

    pub fn fonte_para_repetir(&self, id: &str) -> Res<Option<PathBuf>> {
        Ok(self
            .caminhos
            .lock()
            .unwrap()
            .get(id)
            .filter(|p| p.is_file())
            .cloned())
    }

    /// Nova versão do mesmo documento. A anterior continua valendo até a nova ser publicada.
    pub fn repetir(&self, id: &str, caminho: &Path) -> Res<()> {
        if !decodificacao::suportado(caminho) {
            return Err("Formato não suportado.".into());
        }
        let nome = self
            .leitor()
            .documento(id)?
            .ok_or("Documento inexistente")?
            .nome;
        self.enfileirar(id.to_string(), nome, caminho.to_path_buf());
        self.mudou();
        Ok(())
    }

    fn atualizar_item(&self, doc: &str, fase: &'static str, progresso: Option<f64>) {
        let mut fila = self.fila.lock().unwrap();
        if let Some(i) = fila.iter_mut().find(|i| i.doc == doc) {
            i.fase = fase;
            // Frações do mesmo trabalho nunca voltam.
            if let Some(p) = progresso {
                i.progresso = Some(i.progresso.map_or(p, |a| a.max(p)));
            }
        }
    }

    fn processar_arquivo(&self, t: Tarefa) {
        if t.cancelado.load(Ordering::Acquire) {
            return;
        }
        self.atualizar_item(&t.doc, "decodificando", None);
        self.mudou();
        let resultado = self.transcrever_arquivo(&t);
        let cancelado = t.cancelado.load(Ordering::Acquire);
        self.fila
            .lock()
            .unwrap()
            .retain(|i| i.doc != t.doc || !Arc::ptr_eq(&i.cancelado, &t.cancelado));
        let (fim, motivo) = match &resultado {
            Ok(_) => ("concluido", None),
            Err(_) if cancelado => ("cancelado", None),
            Err(e) => ("erro", Some(e.to_string())),
        };
        if let Err(e) = &resultado {
            if !cancelado {
                nota(format!("arquivo {}: falhou ({e})", t.doc));
            }
        }
        (self.avisar)(
            "ditado:resultado",
            Some(json!({"id": t.doc, "resultado": fim, "motivo": motivo})),
        );
        self.mudou();
    }

    fn transcrever_arquivo(&self, t: &Tarefa) -> Res<Desfecho> {
        let fala = self.motores.fala(true)?;
        let falantes = self.motores.falantes();
        let mut armazem = self.armazem()?;
        let versao = armazem.criar_versao(&t.doc, &t.nome, "arquivo")?;
        let encerrar = |armazem: &ArmazemLancamento, estado: &str, erro: Option<&str>| {
            let _ = armazem.encerrar_versao(&versao.id, estado, erro);
        };
        let assinatura = |p: &Path| {
            std::fs::metadata(p)
                .ok()
                .map(|m| (m.len(), m.modified().ok()))
        };
        let original = assinatura(&t.caminho);
        let pcm = match decodificacao::decodificar(&t.caminho) {
            Ok(p) if !t.cancelado.load(Ordering::Acquire) => p,
            Ok(_) => {
                encerrar(&armazem, "cancelada", None);
                return Err("cancelado".into());
            }
            Err(_) => {
                encerrar(
                    &armazem,
                    "falhou",
                    Some("Não consegui ler o áudio deste arquivo."),
                );
                return Err("Não consegui ler o áudio deste arquivo.".into());
            }
        };
        // Sem o WAV (disco cheio), o arquivo ainda é transcrito: só os falantes ficam de fora.
        let mut wav = falantes
            .as_ref()
            .map(|_| self.trabalho.join(format!("falantes-{}.wav", versao.id)));
        if let Some(w) = wav.clone() {
            if let Err(e) = motores::audio::gravar_wav(&w, &pcm) {
                nota(format!(
                    "arquivo {}: wav de falantes não gravado ({e}); segue sem falantes",
                    t.doc
                ));
                let _ = std::fs::remove_file(&w);
                wav = None;
            }
        }
        let falantes = if wav.is_some() { falantes } else { None };
        let preparar = self.preparar.clone();
        let cancelado = t.cancelado.clone();
        let caminho = t.caminho.clone();
        let conferir = || -> Result<(), nucleo::motores::Erro> {
            if cancelado.load(Ordering::Acquire) {
                return Err("cancelado".into());
            }
            if assinatura(&caminho) != original {
                return Err("O arquivo mudou durante a transcrição. Tente de novo.".into());
            }
            Ok(())
        };
        // Evento de progresso (pílula) a cada 250 ms; o estado completo da interface, que relê a
        // lista inteira do SQLite, no máximo uma vez por segundo, além das trocas de etapa e do fim.
        let mut evento = Limitador::new(Duration::from_millis(250));
        let mut estado = Limitador::new(Duration::from_secs(1));
        let mut etapa_anterior = String::new();
        let longo = pcm.len() > 16_000 * 600;
        let progresso = |etapa: &str, fracao: Option<f64>| {
            let (fase, rotulo) = rotulo_de_etapa(etapa);
            self.atualizar_item(&t.doc, fase, fracao);
            let fim = fracao == Some(1.0);
            let trocou = etapa != etapa_anterior;
            if trocou {
                etapa_anterior = etapa.to_string();
            }
            let agora = Instant::now();
            if evento.deixar(agora, fim || trocou) {
                (self.avisar)(
                    "ditado:progresso",
                    Some(progresso_json(
                        &versao.id,
                        "processando",
                        etapa,
                        rotulo,
                        fracao,
                        longo,
                        false,
                    )),
                );
            }
            if estado.deixar(agora, fim || trocou) {
                self.mudou();
            }
        };
        let reconhecedor: &dyn ReconhecedorDeFala = &*fala;
        let resultado = ProcessadorDeArquivo {
            armazem: &mut armazem,
            reconhecedor,
            identificador: falantes.as_deref(),
        }
        .transcrever(
            &versao,
            &pcm,
            wav.as_deref(),
            |texto| preparar(texto),
            conferir,
            progresso,
        );
        if let Some(w) = &wav {
            let _ = std::fs::remove_file(w);
        }
        match resultado {
            Ok(d) => Ok(d),
            Err(e) => {
                let cancelou = t.cancelado.load(Ordering::Acquire);
                encerrar(
                    &armazem,
                    if cancelou { "cancelada" } else { "falhou" },
                    (!cancelou).then(|| e.to_string()).as_deref(),
                );
                Err(e)
            }
        }
    }

    /// Cancela o trabalho em memória. Arquivo novo sem versão publicada sai da lista; com versão
    /// anterior, a anterior continua.
    pub fn cancelar(&self, id: &str) -> Res<()> {
        let mut fila = self.fila.lock().unwrap();
        let mut achou = false;
        for i in fila.iter().filter(|i| i.doc == id) {
            i.cancelado.store(true, Ordering::Release);
            achou = true;
        }
        fila.retain(|i| i.doc != id || (i.fase != "naFila" && i.fase != "formatoNaoSuportado"));
        drop(fila);
        if achou {
            let mut leitor = self.leitor();
            if leitor.versao_ativa(id)?.is_none() && leitor.existe_documento(id)? {
                leitor.descartar(id)?;
            }
        }
        self.mudou();
        Ok(())
    }

    pub fn cancelar_fila(&self) -> Res<()> {
        let ids: Vec<String> = self
            .fila
            .lock()
            .unwrap()
            .iter()
            .map(|i| i.doc.clone())
            .collect();
        for id in ids {
            self.cancelar(&id)?;
        }
        Ok(())
    }

    /// Esconde arquivo ou reunião arquivada (reversível com `desfazer`).
    pub fn remover(&self, id: &str) -> Res<()> {
        if self
            .reuniao
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|a| a.doc == id)
        {
            return Err("Encerre a reunião antes de remover.".into());
        }
        if self.fila.lock().unwrap().iter().any(|i| i.doc == id) {
            for i in self.fila.lock().unwrap().iter().filter(|i| i.doc == id) {
                i.cancelado.store(true, Ordering::Release);
            }
            self.fila
                .lock()
                .unwrap()
                .retain(|i| i.doc != id || (i.fase != "naFila" && i.fase != "formatoNaoSuportado"));
        }
        let leitor = self.leitor();
        if leitor.existe_documento(id)? {
            leitor.definir(id, CampoDocumento::Oculto(true))?;
        }
        drop(leitor);
        self.mudou();
        Ok(())
    }

    pub fn desfazer(&self, id: &str) -> Res<()> {
        self.leitor().definir(id, CampoDocumento::Oculto(false))?;
        self.mudou();
        Ok(())
    }

    // MARK: Edição

    pub fn alterar_titulo(&self, id: &str, nome: &str) -> Res<()> {
        self.leitor()
            .definir(id, CampoDocumento::Nome(nome.to_string()))?;
        self.mudou();
        Ok(())
    }

    /// Nomes iguais (comparação em português, sem acento) unem as duas vozes.
    pub fn renomear_voz(&self, id: &str, voz: &str, nome: &str) -> Res<()> {
        self.leitor().renomear(id, voz, nome)?;
        self.mudou();
        Ok(())
    }

    pub fn reatribuir(
        &self,
        id: &str,
        trecho: &str,
        voz: Option<&str>,
        nome: Option<&str>,
    ) -> Res<()> {
        let mut leitor = self.leitor();
        match (voz, nome) {
            (Some(v), _) => leitor.reatribuir(id, trecho, v)?,
            (None, Some(n)) => leitor.atribuir_nova_pessoa(id, trecho, n)?,
            _ => return Err("falta voz ou nome".into()),
        }
        drop(leitor);
        self.mudou();
        Ok(())
    }

    // MARK: Reunião

    /// `fonte`: "mac"/"sistema" (só o áudio do computador) ou "macEMicrofone"/"sistemaEMicrofone".
    pub fn iniciar_reuniao(self: &Arc<Self>, fonte: &str) -> Res<String> {
        let mut atual = self.reuniao.lock().unwrap();
        if self.desligando.load(Ordering::Acquire) {
            return Err("O Dita.se está fechando.".into());
        }
        if atual.is_some() {
            return Err("Já há uma reunião em andamento.".into());
        }
        if self.finalizando.lock().unwrap().is_some() {
            return Err("Espere a reunião anterior terminar de processar.".into());
        }
        let fala = self
            .motores
            .fala(false)
            .map_err(|e| format!("Não dá para começar agora: {e}"))?;
        let com_microfone = fonte.ends_with("EMicrofone");
        let doc = uuid();
        self.leitor()
            .criar_documento(&doc, "reuniao", "Reunião", Some(fonte), "gravando")?;
        let relogio = Instant::now();
        let vivo = Arc::new(Vivo {
            pausada: AtomicBool::new(false),
            nivel: Mutex::new(0.0),
            ultimo_audio: Mutex::new(relogio),
            pausas: Mutex::new((0.0, None)),
            relogio,
        });

        let mut fontes = vec![(Fonte::Sistema, FonteDeAudio::Mac)];
        if com_microfone {
            fontes.push((Fonte::Microfone, FonteDeAudio::Mic));
        }
        let mut coletor =
            ColetorDeReuniao::new(doc.clone(), self.armazem()?, self.disco.clone(), 0.0);
        for (_, f) in &fontes {
            coletor.abrir(*f, 0.0)?;
        }

        let (envio, recebe) = mpsc::channel::<Captado>();
        let (fim_tx, fim_rx) = mpsc::channel::<()>();
        let mut capturas = Vec::new();
        let mut aviso = None;
        let mut microfone_aberto = false;
        for (fonte_casca, fonte_nucleo) in &fontes {
            let (tx, v, f) = (envio.clone(), vivo.clone(), *fonte_nucleo);
            let derivacao = (*fonte_casca == Fonte::Microfone).then(|| self.derivacao.clone());
            let ao_bloco: AoBloco = Box::new(move |t, bloco, nivel| {
                // Ditado durante a reunião: recebe o microfone mesmo com a reunião pausada.
                if let Some(d) = &derivacao {
                    if let Some(ditado) = d.lock().unwrap().ao_bloco.as_mut() {
                        ditado(bloco.clone(), nivel);
                    }
                }
                // Todo bloco vai ao coletor, inclusive com a reunião pausada: a linha do tempo do
                // núcleo recorta a parte pausada pelo tempo. Descartar aqui perdia o trecho de antes
                // da pausa no bloco que a atravessa (lacuna de até 100 ms a cada pausa).
                if !v.pausada.load(Ordering::Acquire) {
                    {
                        let mut n = v.nivel.lock().unwrap();
                        *n = n.max(nivel);
                    }
                    if nivel > 0.05 {
                        *v.ultimo_audio.lock().unwrap() = Instant::now();
                    }
                }
                let _ = tx.send(Captado::Audio(f, t, bloco));
            });
            match self.capturador.abrir(*fonte_casca, relogio, ao_bloco) {
                Ok(g) => {
                    capturas.push(g);
                    if *fonte_casca == Fonte::Microfone {
                        microfone_aberto = true;
                    }
                }
                Err(e) => {
                    nota(format!(
                        "reunião: fonte {} indisponível ({e})",
                        fonte_nucleo.nome()
                    ));
                    aviso = Some(if *fonte_casca == Fonte::Microfone {
                        "O microfone não abriu; só o áudio do computador está sendo gravado."
                    } else {
                        "O áudio do computador não abriu; só o microfone está sendo gravado."
                    });
                }
            }
        }
        if capturas.is_empty() {
            let mut leitor = self.leitor();
            let _ = leitor.descartar(&doc);
            return Err(
                "Nenhuma fonte de áudio abriu. Confira o microfone e a saída de som.".into(),
            );
        }
        if let Some(a) = aviso {
            self.leitor()
                .definir(&doc, CampoDocumento::AvisoCaptura(Some(a.into())))?;
        }

        // Coletor: grava cada bloco em disco antes de qualquer reconhecimento.
        let servico = Arc::downgrade(self);
        let doc_c = doc.clone();
        let coletor_fim = Arc::new(AtomicBool::new(false));
        let fim_coletor = coletor_fim.clone();
        thread::Builder::new()
            .name("ditase.coletor".into())
            .spawn(move || {
                let mut falha = None;
                for m in recebe {
                    let r = match m {
                        Captado::Audio(f, t, a) => coletor.receber(f, t, &a),
                        Captado::Pausar(t) => coletor.pausar(t),
                        Captado::Retomar(t) => {
                            coletor.retomar(t);
                            Ok(())
                        }
                        Captado::Encerrar(t) => {
                            let r = coletor.encerrar(t);
                            if let Err(e) = r {
                                falha = Some(e.to_string());
                            }
                            break;
                        }
                    };
                    if let Err(e) = r {
                        falha = Some(e.to_string());
                        break;
                    }
                }
                if let Some(e) = falha {
                    nota(format!("reunião {doc_c}: gravação em disco falhou ({e})"));
                    let _ = coletor.armazem.definir(
                        &doc_c,
                        CampoDocumento::ErroCaptura(Some(
                            "A gravação em disco falhou. O áudio já gravado pode ser recuperado."
                                .into(),
                        )),
                    );
                    if let Some(s) = servico.upgrade() {
                        s.parar_capturas(&doc_c);
                    }
                }
                drop(coletor);
                fim_coletor.store(true, Ordering::Release);
                let _ = fim_tx.send(());
            })?;

        // Processador: trechos ao vivo a cada segundo; no fim, passada final e resumo.
        let servico = self.clone();
        let (doc_p, vivo_p) = (doc.clone(), vivo.clone());
        thread::Builder::new()
            .name("ditase.reuniao".into())
            .spawn(move || servico.processar_reuniao(&doc_p, fala, Some((vivo_p, fim_rx))))?;

        self.resumo_cancelado.store(false, Ordering::Release);
        *atual = Some(Ativa {
            doc: doc.clone(),
            vivo,
            capturas,
            envio,
            coletor_fim,
            microfone: microfone_aberto,
        });
        drop(atual);
        nota(format!(
            "reunião {doc} iniciada ({})",
            if com_microfone {
                "sistema e microfone"
            } else {
                "sistema"
            }
        ));
        self.mudou();
        Ok(doc)
    }

    /// Tira a reunião em curso e a marca como finalizando sob as duas travas (mesma ordem de
    /// `iniciar_reuniao`): não há instante em que ela não esteja em nenhuma das duas.
    fn tirar_ativa(&self, doc: Option<&str>) -> Option<Ativa> {
        let mut r = self.reuniao.lock().unwrap();
        if doc.is_some_and(|d| r.as_ref().is_none_or(|a| a.doc != d)) {
            return None;
        }
        let a = r.take()?;
        *self.finalizando.lock().unwrap() = Some(a.doc.clone());
        Some(a)
    }

    /// Avisa um ditado que estivesse derivando do microfone da reunião com a captura ainda aberta,
    /// para ele abrir o microfone próprio e emendar pelo relógio sem perder a fala dita enquanto o
    /// microfone abre (N-1). Só depois fecha as fontes da reunião e solta a derivação. Se o
    /// microfone próprio não abriu com o dispositivo ocupado, a segunda tentativa roda com ele já
    /// livre; se nem assim, o ditado encerra com o que tem e avisa (I-2). Nada disso roda sob
    /// trava do serviço.
    fn soltar_capturas(&self, capturas: Vec<Box<dyn Send>>) {
        let ao_acabar = self.derivacao.lock().unwrap().ao_acabar.take();
        let depois = ao_acabar.and_then(|f| f());
        drop(capturas);
        // Fora da trava: soltar o callback fecha a derivação na emenda do ditado.
        let derivado = self.derivacao.lock().unwrap().ao_bloco.take();
        drop(derivado);
        if let Some(f) = depois {
            f();
        }
    }

    /// Fecha as fontes (falha de disco no coletor): a reunião segue para o fim com o que foi gravado.
    fn parar_capturas(&self, doc: &str) {
        if let Some(a) = self.tirar_ativa(Some(doc)) {
            self.soltar_capturas(a.capturas);
            let _ = self
                .leitor()
                .mudar_estado(doc, "processando", &["gravando", "pausada"]);
            self.mudou();
        }
    }

    fn emitir_trechos(
        &self,
        doc: &str,
        novos: &[TrechoNovo],
        leitura: &ArmazemLancamento,
        vivo: Option<&Vivo>,
    ) {
        if novos.is_empty() {
            return;
        }
        let vozes = leitura
            .documento(doc)
            .ok()
            .flatten()
            .map(|d| serde_json::to_value(d.vozes).unwrap_or_default())
            .unwrap_or_else(|| json!([]));
        let duracao = vivo.map_or_else(
            || novos.iter().map(|t| t.fim).fold(0.0, f64::max),
            |v| v.decorrido(),
        );
        (self.avisar)(
            "ditado:trechos",
            Some(
                json!({"id": doc, "trechos": novos.iter().map(trecho_json).collect::<Vec<_>>(), "vozes": vozes, "duracao": duracao, "atraso": 0}),
            ),
        );
    }

    fn processar_reuniao(
        &self,
        doc: &str,
        fala: Arc<dyn crate::integracao::Fala>,
        ao_vivo: Option<(Arc<Vivo>, mpsc::Receiver<()>)>,
    ) {
        let falantes = self.motores.falantes();
        let resultado = (|| -> Res<bool> {
            let mut armazem = self.armazem()?;
            let leitura = self.armazem()?;
            let preparar = self.preparar.clone();
            let preparar = move |t: &str| preparar(t);
            let reconhecedor: &dyn ReconhecedorDeFala = &*fala;
            let mut p = ProcessadorDeReuniao::new(doc, &mut armazem, &self.disco, reconhecedor)
                .com_preparar(&preparar);
            if let Some((vivo, fim)) = &ao_vivo {
                while let Err(mpsc::RecvTimeoutError::Timeout) =
                    fim.recv_timeout(Duration::from_secs(1))
                {
                    match p.bloco_selado() {
                        Ok(novos) => self.emitir_trechos(doc, &novos, &leitura, Some(vivo)),
                        Err(e) => nota(format!("reunião {doc}: bloco ao vivo ({e})")),
                    }
                    self.progresso_ao_vivo(doc, vivo);
                }
                // App fechando: o coletor já gravou tudo; o reconhecimento fica para a recuperação.
                if self.desligando.load(Ordering::Acquire) {
                    return Err(Box::new(Desligando));
                }
                let novos = p.fechar()?;
                self.emitir_trechos(doc, &novos, &leitura, Some(vivo));
            } else {
                let novos = p.recuperar()?;
                self.emitir_trechos(doc, &novos, &leitura, None);
            }
            p.encerrar_costuras()?;
            (self.avisar)(
                "ditado:progresso",
                Some(progresso_json(
                    doc,
                    "processando",
                    "passadaFinal",
                    "Passada final",
                    None,
                    false,
                    false,
                )),
            );
            let identificados = p.passada_final(falantes.as_deref())?;
            Ok(identificados)
        })();
        if let Err(e) = &resultado {
            if e.is::<Desligando>() {
                nota(format!(
                    "reunião {doc}: app fechando; áudio guardado para recuperar"
                ));
                *self.finalizando.lock().unwrap() = None;
                return;
            }
        }
        const FALHOU: &str =
            "O processamento não terminou. O áudio continua guardado para nova tentativa.";
        let motivo = match &resultado {
            Ok(id) => {
                nota(format!(
                    "reunião {doc}: finalizada, falantes {}",
                    if *id {
                        "identificados"
                    } else {
                        "indisponíveis"
                    }
                ));
                if let Ok(a) = self.armazem() {
                    let _ = a.definir(doc, CampoDocumento::Aviso(None));
                }
                None
            }
            Err(e) => {
                nota(format!(
                    "reunião {doc}: processamento incompleto ({e}); áudio mantido para recuperar"
                ));
                // Persistido: some do "finalizando" e aparece em "Reuniões interrompidas" com o motivo, também depois de reabrir.
                if let Ok(a) = self.armazem() {
                    let _ = a.mudar_estado(
                        doc,
                        "interrompida",
                        &["processando", "gravando", "pausada"],
                    );
                    let _ = a.definir(doc, CampoDocumento::Aviso(Some(FALHOU.into())));
                }
                Some(FALHOU.to_string())
            }
        };
        if resultado.is_ok()
            && self.motores.resumo_disponivel()
            && !self.desligando.load(Ordering::Acquire)
        {
            // Cancelado durante a passada final: dispensa o resumo em vez de gerar.
            if self.resumo_cancelado.load(Ordering::Acquire) {
                if let Ok(a) = self.armazem() {
                    let _ = a.definir(doc, CampoDocumento::ResumoCancelado);
                }
            } else {
                self.gerar_resumo(doc);
            }
        }
        *self.finalizando.lock().unwrap() = None;
        (self.avisar)(
            "ditado:resultado",
            Some(
                json!({"id": doc, "resultado": if motivo.is_some() { "erro" } else { "concluido" }, "motivo": motivo}),
            ),
        );
        self.mudou();
    }

    fn progresso_ao_vivo(&self, doc: &str, vivo: &Vivo) {
        let pausada = vivo.pausada.load(Ordering::Acquire);
        let nivel = std::mem::take(&mut *vivo.nivel.lock().unwrap());
        let sem_audio = vivo.ultimo_audio.lock().unwrap().elapsed().as_secs_f64();
        let mut v = json!({
            "id": doc, "fase": if pausada { "pausada" } else { "gravando" },
            "etapa": {"id": "gravando", "rotulo": if pausada { "Reunião pausada" } else { "Reunião em andamento" }},
            "estimativa": false, "longo": false, "reuniaoAtiva": true, "nivel": nivel, "decorrido": vivo.decorrido(), "audioChegando": sem_audio < 3.0,
        });
        if !pausada && sem_audio >= 3.0 {
            v["motivo"] = json!("Nenhum som chegando. Confira o volume e o microfone.");
        }
        (self.avisar)("ditado:progresso", Some(v));
    }

    fn com_ativa<T>(&self, id: &str, f: impl FnOnce(&Ativa) -> T) -> Res<T> {
        let r = self.reuniao.lock().unwrap();
        match r.as_ref() {
            Some(a) if a.doc == id => Ok(f(a)),
            _ => Err("Essa reunião não está em andamento.".into()),
        }
    }

    /// Alterna pausa e gravação. O tempo pausado fica fora da transcrição.
    pub fn pausar(&self, id: &str) -> Res<()> {
        let pausou = self.com_ativa(id, |a| {
            alternar_pausa(&a.vivo, |m| {
                let _ = a.envio.send(m);
            })
        })?;
        let leitor = self.leitor();
        if pausou {
            leitor.mudar_estado(id, "pausada", &["gravando"])?;
        } else {
            leitor.mudar_estado(id, "gravando", &["pausada"])?;
        }
        drop(leitor);
        self.mudou();
        Ok(())
    }

    /// Encerra a captura; a passada final e o resumo seguem em segundo plano.
    pub fn encerrar(&self, id: &str) -> Res<()> {
        let a = self
            .tirar_ativa(Some(id))
            .ok_or("Essa reunião não está em andamento.")?;
        let t = a.vivo.relogio.elapsed().as_secs_f64();
        self.soltar_capturas(a.capturas);
        let _ = a.envio.send(Captado::Encerrar(t));
        let duracao = a.vivo.decorrido();
        let leitor = self.leitor();
        leitor.mudar_estado(id, "processando", &["gravando", "pausada"])?;
        let _ = leitor.definir(id, CampoDocumento::Duracao(duracao));
        drop(leitor);
        nota(format!("reunião {id} encerrada após {duracao:.0} s"));
        self.mudou();
        Ok(())
    }

    /// Reprocessa a gravação preservada de uma reunião interrompida.
    pub fn recuperar_reuniao(self: &Arc<Self>, id: &str) -> Res<()> {
        self.reservar(id)?;
        self.resumo_cancelado.store(false, Ordering::Release);
        let fala = match self.motores.fala(false) {
            Ok(f) => f,
            Err(e) => {
                *self.finalizando.lock().unwrap() = None;
                return Err(e.into());
            }
        };
        let _ =
            self.leitor()
                .mudar_estado(id, "processando", &["gravando", "pausada", "interrompida"]);
        let s = self.clone();
        let doc = id.to_string();
        thread::Builder::new()
            .name("ditase.recuperar".into())
            .spawn(move || s.processar_reuniao(&doc, fala, None))?;
        self.mudou();
        Ok(())
    }

    /// Reserva a vaga única de processamento para `id`, sob as mesmas travas de `tirar_ativa`.
    fn reservar(&self, id: &str) -> Res<()> {
        let r = self.reuniao.lock().unwrap();
        if self.desligando.load(Ordering::Acquire) {
            return Err("O Dita.se está fechando.".into());
        }
        if r.is_some() {
            return Err("Encerre a reunião em andamento antes.".into());
        }
        let mut f = self.finalizando.lock().unwrap();
        if f.is_some() {
            return Err("Espere o processamento atual terminar.".into());
        }
        *f = Some(id.to_string());
        Ok(())
    }

    pub fn descartar_reuniao(&self, id: &str) -> Res<()> {
        if self.reuniao_atual().as_deref() == Some(id) {
            return Err("Essa reunião está em andamento.".into());
        }
        let mut leitor = self.leitor();
        leitor.descartar(id)?;
        leitor.limpar(id, &self.disco)?;
        drop(leitor);
        self.disco.remover_pasta(id);
        self.mudou();
        Ok(())
    }

    // MARK: Resumo

    /// Não zera o cancelamento: quem reserva (encerrar, recuperar, resumir) zera antes.
    fn gerar_resumo(&self, doc: &str) {
        self.resumindo.store(true, Ordering::Release);
        self.mudou();
        self.gerar_resumo_dentro(doc);
        self.resumindo.store(false, Ordering::Release);
    }

    fn gerar_resumo_dentro(&self, doc: &str) {
        let Ok(Some(d)) = self.armazem().and_then(|a| a.documento(doc)) else {
            return;
        };
        if d.trechos.is_empty() {
            return;
        }
        let nomes: HashMap<&str, &str> = d
            .vozes
            .iter()
            .map(|v| (v.id.as_str(), v.nome.as_str()))
            .collect();
        let linhas: Vec<String> = d
            .trechos
            .iter()
            .map(|t| {
                format!(
                    "{}: {}",
                    nomes.get(t.voz.as_str()).copied().unwrap_or(&t.voz),
                    t.texto
                )
            })
            .collect();
        (self.avisar)(
            "ditado:progresso",
            Some(progresso_json(
                doc,
                "processando",
                "resumindo",
                "Gerando resumo",
                None,
                false,
                false,
            )),
        );
        let cancelado = self.resumo_cancelado.clone();
        let r = self
            .motores
            .resumir(&linhas, &|| cancelado.load(Ordering::Acquire));
        let Ok(armazem) = self.armazem() else { return };
        let campo = match r {
            _ if self.resumo_cancelado.load(Ordering::Acquire) => CampoDocumento::ResumoCancelado,
            Ok(topicos) if !topicos.is_empty() => CampoDocumento::Resumo(topicos),
            Ok(_) => CampoDocumento::ErroResumo("O resumo veio vazio.".into()),
            Err(e) => {
                nota(format!("reunião {doc}: resumo falhou"));
                CampoDocumento::ErroResumo(e)
            }
        };
        if let Err(e) = armazem.definir(doc, campo) {
            nota(format!("reunião {doc}: resumo não gravado ({e})"));
        }
    }

    /// Gera de novo; a reunião aparece como "finalizando" e o resumo anterior fica até sair o novo.
    pub fn resumir(self: &Arc<Self>, id: &str) -> Res<()> {
        if !self.motores.resumo_disponivel() {
            return Err(
                "Ligue a IA local nos Ajustes e baixe o modelo para gerar o resumo.".into(),
            );
        }
        self.reservar(id)?;
        self.resumo_cancelado.store(false, Ordering::Release);
        let s = self.clone();
        let doc = id.to_string();
        thread::Builder::new()
            .name("ditase.resumo".into())
            .spawn(move || {
                s.gerar_resumo(&doc);
                *s.finalizando.lock().unwrap() = None;
                s.mudou();
            })?;
        self.mudou();
        Ok(())
    }

    pub fn cancelar_resumo(&self, id: &str) {
        if self.finalizando.lock().unwrap().as_deref() == Some(id) {
            self.resumo_cancelado.store(true, Ordering::Release);
        }
    }

    /// Fecha as fontes e espera, até `prazo`, o coletor gravar em disco o que ainda estava em
    /// memória. Não espera reconhecimento, passada final nem resumo: isso fica para a recuperação
    /// na próxima abertura (a reunião aparece em `recuperaveis`).
    ///
    /// Também desfaz o ciclo entre o serviço e as threads da reunião (o canal do coletor fica na
    /// reunião em curso e a thread de processamento segura o serviço).
    pub fn desligar(&self, prazo: Duration) {
        self.desligando.store(true, Ordering::Release);
        self.resumo_cancelado.store(true, Ordering::Release);
        let limite = Instant::now() + prazo;
        if let Some(a) = self.tirar_ativa(None) {
            let t = a.vivo.relogio.elapsed().as_secs_f64();
            self.soltar_capturas(a.capturas);
            let _ = a.envio.send(Captado::Encerrar(t));
            while !a.coletor_fim.load(Ordering::Acquire) && Instant::now() < limite {
                thread::sleep(Duration::from_millis(10));
            }
            if !a.coletor_fim.load(Ordering::Acquire) {
                nota(format!(
                    "reunião {}: o coletor não terminou no prazo de saída",
                    a.doc
                ));
            }
        }
        self.cancelar_fila().ok();
    }

    /// Para os testes: espera as threads de processamento soltarem o serviço.
    #[cfg(test)]
    pub fn esperar_ocioso(&self, prazo: Duration) -> bool {
        let limite = Instant::now() + prazo;
        let ocioso =
            || self.finalizando.lock().unwrap().is_none() && self.fila.lock().unwrap().is_empty();
        while !ocioso() && Instant::now() < limite {
            thread::sleep(Duration::from_millis(10));
        }
        ocioso()
    }
}

#[cfg(test)]
pub mod dubles {
    use std::{
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
        thread,
        time::Instant,
    };

    use super::{AoBloco, Capturador};
    use crate::{audio::Fonte, integracao::dubles::voz};

    /// Dublê de captura: entrega `segundos` de voz em blocos de 100 ms, com o tempo pela contagem de amostras.
    /// Voz em tempo real: um bloco de 100 ms a cada 100 ms, com o tempo pelo relógio da reunião
    /// (mesmo cálculo da captura real), até ser solta.
    pub struct CapturaEmTempoReal;
    impl Capturador for CapturaEmTempoReal {
        fn abrir(
            &self,
            _f: Fonte,
            relogio: Instant,
            mut ao_bloco: AoBloco,
        ) -> Result<Box<dyn Send>, String> {
            let fim = Arc::new(AtomicBool::new(false));
            let p = fim.clone();
            let fio = thread::spawn(move || {
                let bloco = voz(0.1, 0.0);
                let mut tempo = super::RelogioDeFonte::default();
                while !p.load(Ordering::Acquire) {
                    thread::sleep(std::time::Duration::from_millis(100));
                    ao_bloco(
                        tempo.tempo(relogio.elapsed().as_secs_f64(), bloco.len()),
                        bloco.clone(),
                        0.3,
                    );
                }
            });
            struct Guarda(Arc<AtomicBool>, Option<thread::JoinHandle<()>>);
            impl Drop for Guarda {
                fn drop(&mut self) {
                    self.0.store(true, Ordering::Release);
                    if let Some(f) = self.1.take() {
                        let _ = f.join();
                    }
                }
            }
            Ok(Box::new(Guarda(fim, Some(fio))))
        }
    }

    pub struct CapturaDuble {
        pub segundos: f64,
    }
    impl Capturador for CapturaDuble {
        fn abrir(
            &self,
            fonte: Fonte,
            _relogio: Instant,
            mut ao_bloco: AoBloco,
        ) -> Result<Box<dyn Send>, String> {
            let pcm = if fonte == Fonte::Sistema {
                voz(self.segundos, 0.6)
            } else {
                voz(1.0, self.segundos - 0.4)
            };
            let fim = Arc::new(AtomicBool::new(false));
            let parar = fim.clone();
            let fio = thread::spawn(move || {
                for (i, bloco) in pcm.chunks(1600).enumerate() {
                    if parar.load(Ordering::Acquire) {
                        break;
                    }
                    ao_bloco(i as f64 * 0.1, bloco.to_vec(), 0.3);
                }
            });
            struct Guarda(Arc<AtomicBool>, Option<thread::JoinHandle<()>>);
            impl Drop for Guarda {
                fn drop(&mut self) {
                    self.0.store(true, Ordering::Release);
                    if let Some(f) = self.1.take() {
                        let _ = f.join();
                    }
                }
            }
            Ok(Box::new(Guarda(fim, Some(fio))))
        }
    }
}

#[cfg(test)]
mod testes {
    use super::{dubles::CapturaDuble, *};
    use crate::integracao::dubles::{voz, MotoresDuble};

    struct Registro(Mutex<Vec<(String, Option<Value>)>>);

    fn servico(segundos: f64) -> (tempfile::TempDir, Arc<Lancamento>, Arc<Registro>) {
        servico_com(segundos, MotoresDuble::novo())
    }

    fn servico_com(
        segundos: f64,
        motores: Arc<MotoresDuble>,
    ) -> (tempfile::TempDir, Arc<Lancamento>, Arc<Registro>) {
        let tmp = crate::caminhos::pasta_de_teste();
        let reg = Arc::new(Registro(Mutex::new(Vec::new())));
        let r = reg.clone();
        let avisar: Avisar = Arc::new(move |a, v| r.0.lock().unwrap().push((a.to_string(), v)));
        let preparar: Preparar = Arc::new(|t: &str| nucleo::RegrasFala::filtrar(t));
        let s = Lancamento::new(
            &tmp.path().join("t.sqlite"),
            &tmp.path().join("Audio"),
            motores,
            Arc::new(CapturaDuble { segundos }),
            preparar,
            avisar,
        )
        .unwrap();
        (tmp, s, reg)
    }

    fn esperar(mut condicao: impl FnMut() -> bool, o_que: &str) {
        let inicio = Instant::now();
        while !condicao() {
            assert!(
                inicio.elapsed() < Duration::from_secs(30),
                "tempo esgotado esperando {o_que}"
            );
            thread::sleep(Duration::from_millis(50));
        }
    }

    fn wav(pasta: &Path, nome: &str, pcm: &[f32]) -> PathBuf {
        let p = pasta.join(nome);
        let mut w = crate::audio::EscritorWav::criar(&p).unwrap();
        w.gravar(pcm).unwrap();
        p
    }

    /// Invalidações do estado completo durante um arquivo: de uma a cada 250 ms para uma por
    /// segundo (240 para 60 por minuto de processamento), sem perder troca de etapa nem o fim.
    #[test]
    fn limitador_de_invalidacoes() {
        let t0 = Instant::now();
        let mut estado = Limitador::new(Duration::from_secs(1));
        let mut antes = 0;
        let mut depois = 0;
        for i in 0..240u64 {
            antes += 1;
            if estado.deixar(t0 + Duration::from_millis(250 * i), i == 239) {
                depois += 1;
            }
        }
        assert_eq!(antes, 240);
        assert_eq!(depois, 61, "60 por segundo de processamento mais o fim");
        assert!(
            estado.deixar(t0 + Duration::from_millis(59_800), true),
            "forçado sempre passa"
        );
        assert_eq!(
            rotulo_de_etapa("limpando"),
            ("diarizando", "Preparando o texto")
        );
        assert!(
            progresso_json("v", "processando", "diarizando", "x", None, false, false)
                .get("progresso")
                .is_none()
        );
    }

    #[test]
    fn relogio_de_fonte_ignora_oscilacao_e_ressincroniza_em_parada() {
        let mut r = RelogioDeFonte::default();
        assert_eq!(r.tempo(0.1, 1600), 0.0);
        // Chamada atrasada 80 ms: o tempo segue a contagem (0,1), não o relógio (0,18).
        assert!((r.tempo(0.28, 1600) - 0.1).abs() < 1e-9);
        // A seguinte chega adiantada: continua contínua.
        assert!((r.tempo(0.30, 1600) - 0.2).abs() < 1e-9);
        // Parada real de 1 s: ressincroniza.
        assert!((r.tempo(1.5, 1600) - 1.4).abs() < 1e-9);
        assert!((r.tempo(1.6, 1600) - 1.5).abs() < 1e-9);
    }

    /// I2: na retomada o `Retomar` entra no canal enquanto a captura ainda está pausada (nenhum
    /// bloco de depois chega antes dele); na pausa, a captura já parou quando o `Pausar` entra.
    #[test]
    fn pausa_e_retomada_mandam_a_mensagem_na_ordem_certa() {
        let vivo = Vivo {
            pausada: AtomicBool::new(false),
            nivel: Mutex::new(0.0),
            ultimo_audio: Mutex::new(Instant::now()),
            pausas: Mutex::new((0.0, None)),
            relogio: Instant::now(),
        };
        assert!(alternar_pausa(&vivo, |m| {
            assert!(matches!(m, Captado::Pausar(_)));
            assert!(
                vivo.pausada.load(Ordering::Acquire),
                "a captura ainda enviava quando o Pausar entrou"
            );
        }));
        assert!(!alternar_pausa(&vivo, |m| {
            assert!(matches!(m, Captado::Retomar(_)));
            assert!(
                vivo.pausada.load(Ordering::Acquire),
                "a captura voltou a enviar antes do Retomar"
            );
        }));
        assert!(!vivo.pausada.load(Ordering::Acquire));
    }

    /// I2 de ponta a ponta: retomadas seguidas com áudio chegando não geram lacuna.
    #[test]
    fn pausas_seguidas_nao_geram_lacuna() {
        let tmp = crate::caminhos::pasta_de_teste();
        let s = Lancamento::new(
            &tmp.path().join("t.sqlite"),
            &tmp.path().join("Audio"),
            MotoresDuble::novo(),
            Arc::new(dubles::CapturaEmTempoReal),
            Arc::new(|t: &str| t.to_string()),
            Arc::new(|_, _| {}),
        )
        .unwrap();
        let id = s.iniciar_reuniao("mac").unwrap();
        for _ in 0..8 {
            thread::sleep(Duration::from_millis(130));
            s.pausar(&id).unwrap();
            thread::sleep(Duration::from_millis(70));
            s.pausar(&id).unwrap();
        }
        thread::sleep(Duration::from_millis(300));
        s.encerrar(&id).unwrap();
        esperar(|| s.documento(&id).unwrap()["fase"] == "pronta", "fim");
        let doc = s.documento(&id).unwrap();
        assert!(
            doc.get("lacunas")
                .is_none_or(|l| l.as_array().is_none_or(|l| l.is_empty())),
            "{doc}"
        );
    }

    /// I3: cancelar durante a passada final dispensa o resumo (antes, gerar_resumo zerava o pedido).
    #[test]
    fn cancelar_resumo_na_passada_final_dispensa_o_resumo() {
        let (_tmp, s, _) = servico_com(3.0, MotoresDuble::com(|m| m.com_limpeza = true));
        let id = s.iniciar_reuniao("mac").unwrap();
        thread::sleep(Duration::from_millis(300));
        s.encerrar(&id).unwrap();
        s.cancelar_resumo(&id);
        esperar(
            || s.esperar_ocioso(Duration::from_millis(10)),
            "fim do processamento",
        );
        let doc = s.documento(&id).unwrap();
        assert_eq!(doc["resumoCancelado"], true, "{doc}");
        assert!(doc.get("resumo").is_none_or(Value::is_null), "{doc}");
    }

    /// I3: durante o resumo a reunião mostra `etapa: resumindo` (a interface só mostra "Cancelar
    /// resumo" assim) e cancelar ali grava `resumoCancelado`.
    #[test]
    fn resumo_em_andamento_aparece_como_resumindo_e_cancela() {
        let (_tmp, s, reg) = servico_com(
            3.0,
            MotoresDuble::com(|m| {
                m.com_limpeza = true;
                m.resumo_lento = true;
            }),
        );
        let id = s.iniciar_reuniao("mac").unwrap();
        thread::sleep(Duration::from_millis(300));
        s.encerrar(&id).unwrap();
        esperar(
            || s.estado(true).unwrap()["reuniao"]["etapa"] == "resumindo",
            "etapa resumindo",
        );
        // O estado muda um instante antes de o evento sair: espera os dois.
        esperar(
            || {
                reg.0.lock().unwrap().iter().any(|(a, v)| {
                    a == "ditado:progresso" && v.as_ref().unwrap()["etapa"]["id"] == "resumindo"
                })
            },
            "evento resumindo",
        );
        s.cancelar_resumo(&id);
        esperar(
            || s.esperar_ocioso(Duration::from_millis(10)),
            "fim do resumo",
        );
        assert_eq!(s.documento(&id).unwrap()["resumoCancelado"], true);
    }

    /// I6: enquanto a reunião sai de "em curso" para "finalizando", ela nunca fica fora das duas:
    /// descartar e recuperar pela ponte, ao mesmo tempo, não a pegam no meio.
    #[test]
    fn transicao_para_finalizando_nao_abre_brecha() {
        for _ in 0..10 {
            let (_tmp, s, _) = servico(2.0);
            let id = s.iniciar_reuniao("mac").unwrap();
            let fim = Arc::new(AtomicBool::new(false));
            let (s2, id2, f2) = (s.clone(), id.clone(), fim.clone());
            let intruso = thread::spawn(move || {
                let mut entrou = 0;
                while !f2.load(Ordering::Acquire) {
                    if s2.descartar_reuniao(&id2).is_ok() {
                        entrou += 1;
                    }
                    if s2.recuperar_reuniao(&id2).is_ok() {
                        entrou += 1;
                    }
                }
                entrou
            });
            thread::sleep(Duration::from_millis(20));
            s.encerrar(&id).unwrap();
            thread::sleep(Duration::from_millis(20));
            fim.store(true, Ordering::Release);
            assert_eq!(
                intruso.join().unwrap(),
                0,
                "descartar ou recuperar entrou na reunião em transição"
            );
            esperar(|| s.esperar_ocioso(Duration::from_millis(10)), "fim");
        }
    }

    /// M4: falha depois de encerrar fica persistida: estado interrompida e motivo em erroProcessamento.
    #[test]
    fn falha_no_processamento_persiste_interrompida_com_motivo() {
        let (_tmp, s, _) = servico_com(3.0, MotoresDuble::com(|m| m.fala_quebrada = true));
        let id = s.iniciar_reuniao("mac").unwrap();
        thread::sleep(Duration::from_millis(300));
        s.encerrar(&id).unwrap();
        esperar(|| s.esperar_ocioso(Duration::from_millis(10)), "fim");
        let estado = s.estado(true).unwrap();
        let r = estado["recuperaveis"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"] == json!(id))
            .cloned()
            .unwrap_or_else(|| panic!("não está em recuperáveis: {estado}"));
        assert_eq!(r["fase"], "interrompida");
        assert!(r["erroProcessamento"]
            .as_str()
            .unwrap()
            .contains("não terminou"));
        assert!(estado.get("reuniao").is_none());
    }

    /// I1: sair com a reunião gravando volta em poucos milissegundos (sem esperar a passada final),
    /// e a reunião fica inteira em disco para a recuperação na próxima abertura.
    #[test]
    fn desligar_espera_so_o_coletor_e_deixa_a_reuniao_recuperavel() {
        let (tmp, s, _) = servico(3.0);
        let id = s.iniciar_reuniao("mac").unwrap();
        thread::sleep(Duration::from_millis(300));
        let inicio = Instant::now();
        s.desligar(Duration::from_secs(2));
        assert!(
            inicio.elapsed() < Duration::from_millis(1500),
            "desligar esperou {:?}",
            inicio.elapsed()
        );
        assert!(
            s.iniciar_reuniao("mac").is_err(),
            "nada novo começa depois de desligar"
        );
        assert!(s.esperar_ocioso(Duration::from_secs(5)));
        drop(s);
        let avisar: Avisar = Arc::new(|_, _| {});
        let s = Lancamento::new(
            &tmp.path().join("t.sqlite"),
            &tmp.path().join("Audio"),
            MotoresDuble::novo(),
            Arc::new(CapturaDuble { segundos: 1.0 }),
            Arc::new(|t: &str| t.to_string()),
            avisar,
        )
        .unwrap();
        assert!(s.estado(true).unwrap()["recuperaveis"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["id"] == json!(id)));
        s.recuperar_reuniao(&id).unwrap();
        esperar(
            || s.documento(&id).unwrap()["fase"] == "pronta",
            "recuperação",
        );
        let palavras: usize = s.documento(&id).unwrap()["trechos"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["texto"].as_str().unwrap().split_whitespace().count())
            .sum();
        assert_eq!(palavras, 6, "3 s de voz = 6 palavras");
    }

    /// M2: sem conseguir gravar o WAV dos falantes, o arquivo ainda é transcrito (sem falantes).
    #[test]
    fn falha_no_wav_dos_falantes_nao_descarta_a_transcricao() {
        let (tmp, s, _) = servico(3.0);
        std::fs::remove_dir_all(tmp.path().join("Audio").join("Trabalho")).unwrap();
        let arq = wav(tmp.path(), "aula.wav", &voz(2.0, 0.5));
        let id = s.adicionar(&[arq]).unwrap().unwrap();
        esperar(|| s.esperar_ocioso(Duration::from_millis(10)), "fim");
        let doc = s.documento(&id).unwrap();
        assert_eq!(doc["fase"], "pronto", "{doc}");
        assert!(!doc["trechos"].as_array().unwrap().is_empty());
    }

    #[test]
    fn arquivo_passa_pelo_processador_com_falantes_e_fica_pronto() {
        let (tmp, s, reg) = servico(3.0);
        assert_eq!(s.estado(true).unwrap()["disponivel"], true);
        let arq = wav(tmp.path(), "pauta.wav", &voz(3.0, 1.0));
        let id = s.adicionar(&[arq]).unwrap().unwrap();
        esperar(
            || s.estado(true).unwrap()["arquivos"][0]["fase"] == "pronto",
            "arquivo pronto",
        );
        let doc = s.documento(&id).unwrap();
        assert_eq!(doc["nome"], "pauta.wav");
        assert!(!doc["trechos"].as_array().unwrap().is_empty());
        assert!(
            doc["vozes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v["id"] == "offline-1"),
            "{doc}"
        );
        let lista = s.estado(true).unwrap();
        assert!(lista["arquivos"][0]["palavras"].as_u64().unwrap() > 0);
        assert!(reg.0.lock().unwrap().iter().any(
            |(a, v)| a == "ditado:resultado" && v.as_ref().unwrap()["resultado"] == "concluido"
        ));
        let progresso: Vec<f64> = reg
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|(a, _)| a == "ditado:progresso")
            .filter_map(|(_, v)| v.as_ref()?["progresso"].as_f64())
            .collect();
        assert!(
            progresso.windows(2).all(|p| p[1] >= p[0]),
            "progresso voltou: {progresso:?}"
        );
        // Busca, título, falante, remover e desfazer.
        assert_eq!(s.procurar_arquivos("PALAVRA").unwrap()[0], json!(id));
        s.alterar_titulo(&id, "Pauta revisada").unwrap();
        s.renomear_voz(&id, "offline-1", "Ana").unwrap();
        let doc = s.documento(&id).unwrap();
        assert_eq!(doc["nome"], "Pauta revisada");
        assert!(doc["vozes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["nome"] == "Ana"));
        s.remover(&id).unwrap();
        assert!(s.estado(true).unwrap()["arquivos"]
            .as_array()
            .unwrap()
            .is_empty());
        s.desfazer(&id).unwrap();
        assert_eq!(
            s.estado(true).unwrap()["arquivos"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn formato_nao_suportado_e_arquivo_ilegivel() {
        let (tmp, s, _) = servico(3.0);
        let txt = tmp.path().join("notas.txt");
        std::fs::write(&txt, "x").unwrap();
        s.adicionar(&[txt]).unwrap();
        assert_eq!(
            s.estado(true).unwrap()["arquivos"][0]["fase"],
            "formatoNaoSuportado"
        );
        let ruim = tmp.path().join("quebrado.mp3");
        std::fs::write(&ruim, b"isto nao e mp3").unwrap();
        let id = s.adicionar(&[ruim]).unwrap().unwrap();
        esperar(
            || {
                s.estado(true).unwrap()["arquivos"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|a| a["id"] == json!(id) && a["fase"] == "falhou")
            },
            "falha registrada",
        );
    }

    #[test]
    fn reuniao_com_microfone_e_sistema_grava_transcreve_e_identifica_falantes() {
        let (_tmp, s, reg) = servico(4.0);
        let id = s.iniciar_reuniao("macEMicrofone").unwrap();
        assert!(
            s.iniciar_reuniao("mac").is_err(),
            "segunda reunião ao mesmo tempo"
        );
        assert_eq!(s.estado(true).unwrap()["reuniao"]["fase"], "gravando");
        thread::sleep(Duration::from_millis(1500));
        s.encerrar(&id).unwrap();
        esperar(
            || {
                s.estado(true).unwrap()["anteriores"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|r| r["id"] == json!(id))
            },
            "reunião arquivada",
        );
        let doc = s.documento(&id).unwrap();
        assert_eq!(doc["fase"], "pronta");
        assert_eq!(doc["falantes"], "identificados");
        let trechos = doc["trechos"].as_array().unwrap();
        assert!(!trechos.is_empty());
        assert!(trechos.iter().all(|t| t["parcial"] != true));
        assert!(
            s.estado(true).unwrap()["recuperaveis"]
                .as_array()
                .unwrap()
                .is_empty(),
            "áudio liberado depois da passada final"
        );
        let eventos = reg.0.lock().unwrap();
        assert!(
            eventos.iter().any(|(a, _)| a == "ditado:trechos"),
            "trechos ao vivo"
        );
        assert!(eventos.iter().any(
            |(a, v)| a == "ditado:resultado" && v.as_ref().unwrap()["resultado"] == "concluido"
        ));
    }

    #[test]
    fn reuniao_interrompida_fica_recuperavel_e_recupera_sem_perder_fala() {
        let tmp = crate::caminhos::pasta_de_teste();
        let banco = tmp.path().join("t.sqlite");
        let audio = tmp.path().join("Audio");
        let id;
        {
            // Captura direta pelo coletor, sem processar: simula o app fechando no meio.
            let armazem = ArmazemLancamento::new(&banco).unwrap();
            id = uuid();
            armazem
                .criar_documento(&id, "reuniao", "Reunião", Some("mac"), "gravando")
                .unwrap();
            let mut c = ColetorDeReuniao::new(
                id.clone(),
                ArmazemLancamento::new(&banco).unwrap(),
                DiscoDeBlocos::new(audio.clone()),
                0.0,
            );
            c.abrir(FonteDeAudio::Mac, 0.0).unwrap();
            for (i, b) in voz(3.0, 0.5).chunks(1600).enumerate() {
                c.receber(FonteDeAudio::Mac, i as f64 * 0.1, b).unwrap();
            }
            c.selar_tudo().unwrap();
        }
        let avisar: Avisar = Arc::new(|_, _| {});
        let s = Lancamento::new(
            &banco,
            &audio,
            MotoresDuble::novo(),
            Arc::new(CapturaDuble { segundos: 1.0 }),
            Arc::new(|t: &str| t.to_string()),
            avisar,
        )
        .unwrap();
        let rec = s.estado(true).unwrap()["recuperaveis"].clone();
        assert!(
            rec.as_array().unwrap().iter().any(|r| r["id"] == json!(id)),
            "{rec}"
        );
        s.recuperar_reuniao(&id).unwrap();
        esperar(
            || s.documento(&id).unwrap()["fase"] == "pronta",
            "recuperação",
        );
        let doc = s.documento(&id).unwrap();
        let palavras: usize = doc["trechos"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["texto"].as_str().unwrap().split_whitespace().count())
            .sum();
        assert_eq!(palavras, 6, "3 s de voz = 6 palavras do dublê: {doc}");
    }

    #[test]
    fn pausa_alterna_estado_e_uuid_e_valido() {
        let (_tmp, s, _) = servico(6.0);
        let id = s.iniciar_reuniao("mac").unwrap();
        assert_eq!(id.len(), 36);
        assert_eq!(&id[14..15], "4");
        s.pausar(&id).unwrap();
        assert_eq!(s.documento(&id).unwrap()["fase"], "pausada");
        s.pausar(&id).unwrap();
        assert_eq!(s.documento(&id).unwrap()["fase"], "gravando");
        s.encerrar(&id).unwrap();
        esperar(|| s.documento(&id).unwrap()["fase"] == "pronta", "fim");
    }
}

impl crate::controlador::MicrofoneCompartilhado for Lancamento {
    fn derivar(
        &self,
        ao_bloco: crate::controlador::AoBlocoDeAudio,
        ao_acabar: crate::controlador::AoAcabar,
    ) -> Option<Box<dyn crate::controlador::FonteAberta>> {
        let r = self.reuniao.lock().unwrap();
        if !r.as_ref().is_some_and(|a| a.microfone) {
            return None;
        }
        let mut d = self.derivacao.lock().unwrap();
        d.geracao += 1;
        d.ao_bloco = Some(ao_bloco);
        d.ao_acabar = Some(ao_acabar);
        Some(Box::new(FonteDerivada {
            derivacao: self.derivacao.clone(),
            geracao: d.geracao,
        }))
    }
}

#[cfg(test)]
mod testes_derivacao {
    use super::{dubles::CapturaDuble, *};
    use crate::{controlador::MicrofoneCompartilhado, integracao::dubles::MotoresDuble};

    /// Captura contínua: um bloco de 100 ms de silêncio a cada 10 ms até ser solta.
    struct CapturaContinua;
    impl Capturador for CapturaContinua {
        fn abrir(
            &self,
            _f: Fonte,
            _r: Instant,
            mut ao_bloco: AoBloco,
        ) -> Result<Box<dyn Send>, String> {
            let fim = Arc::new(AtomicBool::new(false));
            let p = fim.clone();
            let fio = thread::spawn(move || {
                let mut i = 0;
                while !p.load(Ordering::Acquire) {
                    ao_bloco(i as f64 * 0.1, vec![0.0; 1600], 0.0);
                    i += 1;
                    thread::sleep(Duration::from_millis(10));
                }
            });
            struct Guarda(Arc<AtomicBool>, Option<thread::JoinHandle<()>>);
            impl Drop for Guarda {
                fn drop(&mut self) {
                    self.0.store(true, Ordering::Release);
                    if let Some(f) = self.1.take() {
                        let _ = f.join();
                    }
                }
            }
            Ok(Box::new(Guarda(fim, Some(fio))))
        }
    }

    /// I5 no serviço: com reunião gravando o microfone, a derivação recebe blocos, inclusive com a
    /// reunião pausada; sem microfone na reunião, não há derivação; ao parar, deixa de receber.
    #[test]
    fn derivacao_do_microfone_da_reuniao() {
        let tmp = crate::caminhos::pasta_de_teste();
        let s = Lancamento::new(
            &tmp.path().join("t.sqlite"),
            &tmp.path().join("Audio"),
            MotoresDuble::novo(),
            Arc::new(CapturaContinua),
            Arc::new(|t: &str| t.to_string()),
            Arc::new(|_, _| {}),
        )
        .unwrap();
        assert!(
            s.derivar(Box::new(|_, _| {}), Box::new(|| None)).is_none(),
            "sem reunião não há derivação"
        );
        let id = s.iniciar_reuniao("macEMicrofone").unwrap();
        s.pausar(&id).unwrap();
        let recebido = Arc::new(Mutex::new(0usize));
        let r = recebido.clone();
        let mut fonte = s
            .derivar(
                Box::new(move |b, _| *r.lock().unwrap() += b.len()),
                Box::new(|| None),
            )
            .expect("derivação");
        thread::sleep(Duration::from_millis(300));
        let antes = *recebido.lock().unwrap();
        assert!(
            antes > 0,
            "a derivação não recebeu nada com a reunião pausada"
        );
        fonte.parar();
        thread::sleep(Duration::from_millis(200));
        let depois = *recebido.lock().unwrap();
        thread::sleep(Duration::from_millis(200));
        assert_eq!(
            *recebido.lock().unwrap(),
            depois,
            "continuou recebendo depois de parar"
        );
        s.encerrar(&id).unwrap();
        s.desligar(Duration::from_secs(2));
        let id2 = {
            let s2 = Lancamento::new(
                &tmp.path().join("u.sqlite"),
                &tmp.path().join("Audio2"),
                MotoresDuble::novo(),
                Arc::new(CapturaDuble { segundos: 3.0 }),
                Arc::new(|t: &str| t.to_string()),
                Arc::new(|_, _| {}),
            )
            .unwrap();
            let id = s2.iniciar_reuniao("mac").unwrap();
            assert!(
                s2.derivar(Box::new(|_, _| {}), Box::new(|| None)).is_none(),
                "reunião sem microfone não deriva"
            );
            s2.desligar(Duration::from_secs(2));
            id
        };
        assert!(!id2.is_empty());
    }

    /// I-1 (prova do Dev Par): o ditado 1 já parou, mas a FonteDerivada dele só é solta depois que
    /// o ditado 2 derivou. O Drop atrasado não pode desligar a derivação do ditado 2.
    #[test]
    fn soltar_derivacao_antiga_nao_desliga_a_nova() {
        let tmp = crate::caminhos::pasta_de_teste();
        let s = Lancamento::new(
            &tmp.path().join("t.sqlite"),
            &tmp.path().join("Audio"),
            MotoresDuble::novo(),
            Arc::new(CapturaContinua),
            Arc::new(|t: &str| t.to_string()),
            Arc::new(|_, _| {}),
        )
        .unwrap();
        let id = s.iniciar_reuniao("macEMicrofone").unwrap();
        let mut f1 = s
            .derivar(Box::new(|_, _| {}), Box::new(|| None))
            .expect("derivação 1");
        f1.parar();
        let recebido = Arc::new(Mutex::new(0usize));
        let r = recebido.clone();
        let _f2 = s
            .derivar(
                Box::new(move |b, _| *r.lock().unwrap() += b.len()),
                Box::new(|| None),
            )
            .expect("derivação 2");
        drop(f1);
        let antes = *recebido.lock().unwrap();
        thread::sleep(Duration::from_millis(300));
        let depois = *recebido.lock().unwrap();
        s.encerrar(&id).unwrap();
        s.desligar(Duration::from_secs(2));
        assert!(
            depois > antes,
            "o ditado 2 parou de receber microfone quando a fonte do ditado 1 foi solta"
        );
    }

    /// I-2 no serviço: encerrar a reunião avisa a derivação ativa via `ao_acabar`.
    #[test]
    fn encerrar_reuniao_avisa_derivacao_ativa() {
        let tmp = crate::caminhos::pasta_de_teste();
        let s = Lancamento::new(
            &tmp.path().join("t.sqlite"),
            &tmp.path().join("Audio"),
            MotoresDuble::novo(),
            Arc::new(CapturaContinua),
            Arc::new(|t: &str| t.to_string()),
            Arc::new(|_, _| {}),
        )
        .unwrap();
        let id = s.iniciar_reuniao("macEMicrofone").unwrap();
        let avisou = Arc::new(AtomicBool::new(false));
        let a = avisou.clone();
        let _f = s
            .derivar(
                Box::new(|_, _| {}),
                Box::new(move || {
                    a.store(true, Ordering::Release);
                    None
                }),
            )
            .expect("derivação");
        s.encerrar(&id).unwrap();
        assert!(
            avisou.load(Ordering::Acquire),
            "encerrar a reunião não avisou o ditado derivado"
        );
        s.desligar(Duration::from_secs(2));
    }

    /// N-1 no serviço: o aviso chega com a captura da reunião ainda aberta e a derivação ainda
    /// entregando blocos; a captura só fecha e a derivação só solta depois que o aviso volta, e a
    /// segunda fase roda por último. Antes, a captura fechava primeiro e a fala dita enquanto o
    /// microfone próprio abria se perdia.
    #[test]
    fn encerrar_avisa_com_a_captura_ainda_aberta_e_solta_depois() {
        struct Solto(Arc<Mutex<Vec<&'static str>>>);
        impl Drop for Solto {
            fn drop(&mut self) {
                self.0.lock().unwrap().push("solto");
            }
        }
        let tmp = crate::caminhos::pasta_de_teste();
        let s = Lancamento::new(
            &tmp.path().join("t.sqlite"),
            &tmp.path().join("Audio"),
            MotoresDuble::novo(),
            Arc::new(CapturaContinua),
            Arc::new(|t: &str| t.to_string()),
            Arc::new(|_, _| {}),
        )
        .unwrap();
        let id = s.iniciar_reuniao("macEMicrofone").unwrap();
        let recebido = Arc::new(Mutex::new(0usize));
        let ordem = Arc::new(Mutex::new(Vec::<&'static str>::new()));
        let guarda = Solto(ordem.clone());
        let (r, r2, o1, o2) = (
            recebido.clone(),
            recebido.clone(),
            ordem.clone(),
            ordem.clone(),
        );
        let _f = s
            .derivar(
                Box::new(move |b, _| {
                    let _ = &guarda;
                    *r.lock().unwrap() += b.len();
                }),
                Box::new(move || {
                    let antes = *r2.lock().unwrap();
                    thread::sleep(Duration::from_millis(150));
                    let depois = *r2.lock().unwrap();
                    o1.lock().unwrap().push(if depois > antes {
                        "aviso com captura aberta"
                    } else {
                        "aviso com captura fechada"
                    });
                    Some(Box::new(move || o2.lock().unwrap().push("depois")))
                }),
            )
            .expect("derivação");
        s.encerrar(&id).unwrap();
        assert_eq!(
            *ordem.lock().unwrap(),
            vec!["aviso com captura aberta", "solto", "depois"]
        );
        s.desligar(Duration::from_secs(2));
    }

    /// M-5 (prova do Dev Par): relógio do dispositivo 200 ppm mais rápido que o da reunião.
    #[test]
    fn deriva_do_relogio_do_dispositivo_nao_descarta_fala() {
        let mut relogio = RelogioDeFonte::default();
        let linha = nucleo::LinhaDoTempo::new(0.0);
        let mut cursor = nucleo::CursorDeFonte::new(0);
        let (n, ppm) = (1600usize, 200e-6);
        let (mut aceitas, mut entregues) = (0usize, 0usize);
        for k in 0..36_000u64 {
            let agora = (k + 1) as f64 * 0.1 / (1.0 + ppm);
            let t = relogio.tempo(agora, n);
            for e in cursor.receber(t, n, &linha) {
                if let nucleo::Evento::Amostras(p) = e {
                    aceitas += p.quantidade;
                }
            }
            entregues += n;
        }
        let perdidas = entregues - aceitas;
        assert_eq!(
            perdidas,
            0,
            "o coletor descartou {perdidas} amostras ({:.2} s) de fala nova como repetidas",
            perdidas as f64 / 16_000.0
        );
    }

    /// M-5 (prova do Dev Par): entrega atrasada 0,5 s com blocos acumulados chegando juntos.
    #[test]
    fn atraso_de_entrega_com_fila_nao_descarta_fala() {
        let mut relogio = RelogioDeFonte::default();
        let linha = nucleo::LinhaDoTempo::new(0.0);
        let mut cursor = nucleo::CursorDeFonte::new(0);
        let n = 1600usize;
        let (mut aceitas, mut entregues) = (0usize, 0usize);
        for k in 0..40u64 {
            let fim_real = (k + 1) as f64 * 0.1;
            let agora = if (10..15).contains(&k) { 1.5 } else { fim_real };
            let t = relogio.tempo(agora, n);
            for e in cursor.receber(t, n, &linha) {
                if let nucleo::Evento::Amostras(p) = e {
                    aceitas += p.quantidade;
                }
            }
            entregues += n;
        }
        let perdidas = entregues - aceitas;
        assert_eq!(
            perdidas,
            0,
            "o coletor descartou {perdidas} amostras ({:.2} s) de fala nova",
            perdidas as f64 / 16_000.0
        );
    }
}

#[cfg(test)]
mod medicoes {
    use super::*;
    use crate::integracao::dubles::{voz, MotoresDuble};

    /// Medição (não roda no CI): custo de abrir o serviço com uma reunião de `min` minutos em disco
    /// e de ler o estado para a interface. `cargo test -p ditase medir_abertura -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn medir_abertura() {
        for min in [10usize, 60] {
            let tmp = crate::caminhos::pasta_de_teste();
            let banco = tmp.path().join("t.sqlite");
            let audio = tmp.path().join("Audio");
            {
                let a = ArmazemLancamento::new(&banco).unwrap();
                let id = uuid();
                a.criar_documento(&id, "reuniao", "Reunião", Some("mac"), "gravando")
                    .unwrap();
                let mut c = ColetorDeReuniao::new(
                    id,
                    ArmazemLancamento::new(&banco).unwrap(),
                    DiscoDeBlocos::new(audio.clone()),
                    0.0,
                );
                c.abrir(FonteDeAudio::Mac, 0.0).unwrap();
                let bloco = voz(1.0, 0.0);
                for i in 0..min * 60 {
                    c.receber(FonteDeAudio::Mac, i as f64, &bloco).unwrap();
                }
                c.selar_tudo().unwrap();
            }
            let inicio = Instant::now();
            let s = Lancamento::new(
                &banco,
                &audio,
                MotoresDuble::novo(),
                Arc::new(dubles::CapturaDuble { segundos: 1.0 }),
                Arc::new(|t: &str| t.to_string()),
                Arc::new(|_, _| {}),
            )
            .unwrap();
            let abrir = inicio.elapsed();
            let inicio = Instant::now();
            for _ in 0..20 {
                s.estado(true).unwrap();
            }
            println!(
                "reunião de {min} min em disco: abrir {:.1} ms; lancamento.estado {:.2} ms cada",
                abrir.as_secs_f64() * 1e3,
                inicio.elapsed().as_secs_f64() * 1e3 / 20.0
            );
        }
    }
}
