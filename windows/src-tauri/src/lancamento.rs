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
    sync::{atomic::{AtomicBool, Ordering}, mpsc, Arc, Mutex, MutexGuard},
    thread,
    time::{Duration, Instant},
};

use nucleo::{
    motores::ReconhecedorDeFala,
    ArmazemLancamento, CampoDocumento, ColetorDeReuniao, Desfecho, DiscoDeBlocos, FonteDeAudio, Palavras,
    ProcessadorDeArquivo, ProcessadorDeReuniao, TranscricaoLonga, TrechoNovo,
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
    fn abrir(&self, fonte: Fonte, relogio: Instant, ao_bloco: AoBloco) -> Result<Box<dyn Send>, String>;
}

/// Capturador real: `audio::Captura`. O tempo do bloco é o relógio menos a duração do próprio bloco.
pub struct CapturaDoSistema { pub microfone: Arc<dyn Fn() -> String + Send + Sync> }

impl Capturador for CapturaDoSistema {
    fn abrir(&self, fonte: Fonte, relogio: Instant, mut ao_bloco: AoBloco) -> Result<Box<dyn Send>, String> {
        let nome = if fonte == Fonte::Microfone { (self.microfone)() } else { String::new() };
        let c = crate::audio::Captura::iniciar(fonte, &nome, move |bloco, nivel| {
            let t = relogio.elapsed().as_secs_f64() - bloco.len() as f64 / crate::audio::TAXA as f64;
            ao_bloco(t.max(0.0), bloco, nivel);
        }).map_err(|e| e.to_string())?;
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

struct Tarefa { doc: String, nome: String, caminho: PathBuf, cancelado: Arc<AtomicBool> }

enum Captado { Audio(FonteDeAudio, f64, Vec<f32>), Pausar(f64), Retomar(f64), Encerrar(f64) }

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

struct Ativa { doc: String, vivo: Arc<Vivo>, capturas: Vec<Box<dyn Send>>, envio: mpsc::Sender<Captado> }

pub struct Lancamento {
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
}

fn uuid() -> String {
    use std::hash::{BuildHasher, Hasher};
    use std::sync::atomic::AtomicU64;
    static N: AtomicU64 = AtomicU64::new(0);
    let semente = std::collections::hash_map::RandomState::new();
    let parte = |extra: u64| {
        let mut h = semente.build_hasher();
        h.write_u64(extra);
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
        h.write_u64(N.fetch_add(1, Ordering::Relaxed));
        h.finish()
    };
    let (a, b) = (parte(1), parte(2));
    format!("{:08x}-{:04x}-4{:03x}-{:04x}-{:012x}", a >> 32, (a >> 16) & 0xffff, a & 0xfff, 0x8000 | (b >> 48 & 0x3fff), b & 0xffff_ffff_ffff)
}

fn trecho_json(t: &TrechoNovo) -> Value {
    json!({"id": t.id, "inicio": t.inicio, "fim": t.fim, "voz": t.voz, "texto": t.texto, "parcial": true})
}

impl Lancamento {
    pub fn new(banco: &Path, pasta_audio: &Path, motores: Arc<dyn Motores>, capturador: Arc<dyn Capturador>, preparar: Preparar, avisar: Avisar) -> Res<Arc<Self>> {
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
            banco: banco.to_path_buf(), disco, trabalho, motores, capturador, preparar, avisar,
            leitor: Mutex::new(leitor), fila: Arc::new(Mutex::new(Vec::new())), caminhos: Mutex::new(HashMap::new()),
            envio: Mutex::new(tx), reuniao: Mutex::new(None), finalizando: Arc::new(Mutex::new(None)),
            resumo_cancelado: Arc::new(AtomicBool::new(false)), desligando: AtomicBool::new(false),
        });
        let fraco = Arc::downgrade(&servico);
        thread::Builder::new().name("ditase.arquivos".into()).spawn(move || {
            for t in rx {
                let Some(s) = fraco.upgrade() else { break };
                s.processar_arquivo(t);
            }
        })?;
        Ok(servico)
    }

    fn leitor(&self) -> MutexGuard<'_, ArmazemLancamento> { self.leitor.lock().unwrap_or_else(|e| e.into_inner()) }
    fn mudou(&self) { (self.avisar)("lancamento", None) }

    fn reuniao_atual(&self) -> Option<String> {
        self.reuniao.lock().unwrap().as_ref().map(|a| a.doc.clone()).or_else(|| self.finalizando.lock().unwrap().clone())
    }

    // MARK: Leitura

    pub fn estado(&self, microfone: bool) -> Res<Value> {
        let atual = self.reuniao_atual();
        let leitor = self.leitor();
        let fila = self.fila.lock().unwrap().clone();
        let mut arquivos: Vec<Value> = Vec::new();
        for d in leitor.lista("arquivo")? {
            if d.oculto == Some(true) { continue; }
            let mut v = serde_json::to_value(&d)?;
            if let Some(item) = fila.iter().find(|i| i.doc == d.id) {
                v["fase"] = json!(item.fase);
                v["progresso"] = json!(item.progresso);
                if let Some(m) = &item.motivo { v["motivo"] = json!(m); }
            }
            arquivos.push(v);
        }
        // Itens que nunca chegaram ao banco (formato não suportado).
        let faltando: Vec<&ItemArquivo> = fila.iter().filter(|i| !arquivos.iter().any(|a| a["id"] == json!(i.doc))).collect();
        for item in faltando {
            arquivos.insert(0, json!({"id": item.doc, "nome": item.nome, "criadoEm": crate::integracao::agora(), "vozes": [], "trechos": [], "fase": item.fase, "motivo": item.motivo}));
        }
        let reuniao = match &atual { Some(id) => leitor.documento(id)?, None => None };
        let mut reuniao = reuniao.map(serde_json::to_value).transpose()?;
        if let (Some(r), Some(f)) = (reuniao.as_mut(), self.finalizando.lock().unwrap().as_ref()) {
            if r["id"] == json!(f) { r["fase"] = json!("finalizando"); }
        }
        let anteriores: Vec<TranscricaoLonga> = leitor.lista("reuniao")?.into_iter()
            .filter(|d| d.oculto != Some(true) && Some(&d.id) != atual.as_ref() && d.fase == "pronta").collect();
        let recuperaveis = leitor.recuperaveis(atual.as_deref())?;
        let mut v = json!({
            "arquivos": arquivos, "anteriores": anteriores, "recuperaveis": recuperaveis,
            "permissoes": {"audioSistema": "concedida", "microfone": if microfone { "concedida" } else { "negada" }},
            "disponivel": self.motores.fala_disponivel(),
        });
        if let Some(r) = reuniao { v["reuniao"] = r; }
        Ok(v)
    }

    pub fn documento(&self, id: &str) -> Res<Value> {
        Ok(serde_json::to_value(self.leitor().documento(id)?.ok_or("Documento inexistente")?)?)
    }

    /// IDs dos arquivos cujo texto contém o termo (sem acento, sem diferença de caixa).
    pub fn procurar_arquivos(&self, termo: &str) -> Res<Value> {
        let alvo = Palavras::normalizadas(termo).join(" ");
        if alvo.is_empty() { return Ok(json!([])); }
        let leitor = self.leitor();
        let mut ids = Vec::new();
        for d in leitor.lista("arquivo")? {
            if d.oculto == Some(true) { continue; }
            let Some(completo) = leitor.documento(&d.id)? else { continue };
            let texto = completo.trechos.iter().map(|t| t.texto.as_str()).collect::<Vec<_>>().join(" ");
            if Palavras::normalizadas(&format!("{} {}", completo.nome, texto)).join(" ").contains(&alvo) { ids.push(d.id); }
        }
        Ok(json!(ids))
    }

    // MARK: Arquivos

    fn enfileirar(&self, doc: String, nome: String, caminho: PathBuf) {
        let cancelado = Arc::new(AtomicBool::new(false));
        self.caminhos.lock().unwrap().insert(doc.clone(), caminho.clone());
        let mut fila = self.fila.lock().unwrap();
        fila.retain(|i| i.doc != doc);
        fila.push(ItemArquivo { doc: doc.clone(), nome: nome.clone(), fase: "naFila", progresso: None, motivo: None, cancelado: cancelado.clone() });
        drop(fila);
        let _ = self.envio.lock().unwrap().send(Tarefa { doc, nome, caminho, cancelado });
    }

    /// Põe os arquivos na fila, na ordem. Devolve o ID do primeiro.
    pub fn adicionar(&self, arquivos: &[PathBuf]) -> Res<Option<String>> {
        let mut primeiro = None;
        for caminho in arquivos {
            let doc = uuid();
            let nome = caminho.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "Arquivo".into());
            if !decodificacao::suportado(caminho) {
                self.fila.lock().unwrap().push(ItemArquivo {
                    doc: doc.clone(), nome, fase: "formatoNaoSuportado", progresso: None,
                    motivo: Some("Formato não suportado. Use MP3, M4A, WAV, FLAC, MP4 ou MOV.".into()), cancelado: Arc::new(AtomicBool::new(true)),
                });
            } else {
                self.leitor().criar_documento(&doc, "arquivo", &nome, None, "ativo")?;
                self.enfileirar(doc.clone(), nome, caminho.clone());
            }
            primeiro.get_or_insert(doc);
        }
        self.mudou();
        Ok(primeiro)
    }

    pub fn fonte_para_repetir(&self, id: &str) -> Res<Option<PathBuf>> {
        Ok(self.caminhos.lock().unwrap().get(id).filter(|p| p.is_file()).cloned())
    }

    /// Nova versão do mesmo documento. A anterior continua valendo até a nova ser publicada.
    pub fn repetir(&self, id: &str, caminho: &Path) -> Res<()> {
        if !decodificacao::suportado(caminho) { return Err("Formato não suportado.".into()); }
        let nome = self.leitor().documento(id)?.ok_or("Documento inexistente")?.nome;
        self.enfileirar(id.to_string(), nome, caminho.to_path_buf());
        self.mudou();
        Ok(())
    }

    fn atualizar_item(&self, doc: &str, fase: &'static str, progresso: Option<f64>) {
        let mut fila = self.fila.lock().unwrap();
        if let Some(i) = fila.iter_mut().find(|i| i.doc == doc) {
            i.fase = fase;
            // Frações do mesmo trabalho nunca voltam.
            if let Some(p) = progresso { i.progresso = Some(i.progresso.map_or(p, |a| a.max(p))); }
        }
    }

    fn processar_arquivo(&self, t: Tarefa) {
        if t.cancelado.load(Ordering::Acquire) { return; }
        self.atualizar_item(&t.doc, "decodificando", None);
        self.mudou();
        let resultado = self.transcrever_arquivo(&t);
        let cancelado = t.cancelado.load(Ordering::Acquire);
        self.fila.lock().unwrap().retain(|i| i.doc != t.doc || !Arc::ptr_eq(&i.cancelado, &t.cancelado));
        let (fim, motivo) = match &resultado {
            Ok(_) => ("concluido", None),
            Err(_) if cancelado => ("cancelado", None),
            Err(e) => ("erro", Some(e.to_string())),
        };
        if let Err(e) = &resultado { if !cancelado { nota(format!("arquivo {}: falhou ({e})", t.doc)); } }
        (self.avisar)("ditado:resultado", Some(json!({"id": t.doc, "resultado": fim, "motivo": motivo})));
        self.mudou();
    }

    fn transcrever_arquivo(&self, t: &Tarefa) -> Res<Desfecho> {
        let fala = self.motores.fala(true)?;
        let falantes = self.motores.falantes();
        let mut armazem = ArmazemLancamento::new(&self.banco)?;
        let versao = armazem.criar_versao(&t.doc, &t.nome, "arquivo")?;
        let encerrar = |armazem: &ArmazemLancamento, estado: &str, erro: Option<&str>| { let _ = armazem.encerrar_versao(&versao.id, estado, erro); };
        let assinatura = |p: &Path| std::fs::metadata(p).ok().map(|m| (m.len(), m.modified().ok()));
        let original = assinatura(&t.caminho);
        let pcm = match decodificacao::decodificar(&t.caminho) {
            Ok(p) if !t.cancelado.load(Ordering::Acquire) => p,
            Ok(_) => { encerrar(&armazem, "cancelada", None); return Err("cancelado".into()); }
            Err(_) => {
                encerrar(&armazem, "falhou", Some("Não consegui ler o áudio deste arquivo."));
                return Err("Não consegui ler o áudio deste arquivo.".into());
            }
        };
        let wav = falantes.as_ref().map(|_| self.trabalho.join(format!("falantes-{}.wav", versao.id)));
        if let Some(w) = &wav {
            if let Err(e) = motores::audio::gravar_wav(w, &pcm) { nota(format!("arquivo {}: wav de falantes ({e})", t.doc)); }
        }
        let preparar = self.preparar.clone();
        let cancelado = t.cancelado.clone();
        let caminho = t.caminho.clone();
        let conferir = || -> Result<(), nucleo::motores::Erro> {
            if cancelado.load(Ordering::Acquire) { return Err("cancelado".into()); }
            if assinatura(&caminho) != original { return Err("O arquivo mudou durante a transcrição. Tente de novo.".into()); }
            Ok(())
        };
        let mut ultimo = Instant::now() - Duration::from_secs(1);
        let progresso = |etapa: &str, fracao: Option<f64>| {
            let (fase, rotulo) = match etapa {
                "transcrevendo" => ("transcrevendo", "Transcrevendo"),
                "diarizando" => ("diarizando", "Separando falantes"),
                _ => ("diarizando", "Preparando o texto"),
            };
            self.atualizar_item(&t.doc, fase, fracao);
            if ultimo.elapsed() >= Duration::from_millis(250) || fracao == Some(1.0) {
                ultimo = Instant::now();
                let mut v = json!({"id": versao.id, "fase": "processando", "etapa": {"id": etapa, "rotulo": rotulo}, "estimativa": fracao.is_some(), "longo": pcm.len() > 16_000 * 600, "reuniaoAtiva": false});
                if let Some(f) = fracao { v["progresso"] = json!(f); }
                (self.avisar)("ditado:progresso", Some(v));
                self.mudou();
            }
        };
        let reconhecedor: &dyn ReconhecedorDeFala = &*fala;
        let resultado = ProcessadorDeArquivo { armazem: &mut armazem, reconhecedor, identificador: falantes.as_deref() }
            .transcrever(&versao, &pcm, wav.as_deref(), |texto| preparar(texto), conferir, progresso);
        if let Some(w) = &wav { let _ = std::fs::remove_file(w); }
        match resultado {
            Ok(d) => Ok(d),
            Err(e) => {
                let cancelou = t.cancelado.load(Ordering::Acquire);
                encerrar(&armazem, if cancelou { "cancelada" } else { "falhou" }, (!cancelou).then(|| e.to_string()).as_deref());
                Err(e)
            }
        }
    }

    /// Cancela o trabalho em memória. Arquivo novo sem versão publicada sai da lista; com versão
    /// anterior, a anterior continua.
    pub fn cancelar(&self, id: &str) -> Res<()> {
        let mut fila = self.fila.lock().unwrap();
        let mut achou = false;
        for i in fila.iter().filter(|i| i.doc == id) { i.cancelado.store(true, Ordering::Release); achou = true; }
        fila.retain(|i| i.doc != id || (i.fase != "naFila" && i.fase != "formatoNaoSuportado"));
        drop(fila);
        if achou {
            let mut leitor = self.leitor();
            if leitor.versao_ativa(id)?.is_none() && leitor.existe_documento(id)? { leitor.descartar(id)?; }
        }
        self.mudou();
        Ok(())
    }

    pub fn cancelar_fila(&self) -> Res<()> {
        let ids: Vec<String> = self.fila.lock().unwrap().iter().map(|i| i.doc.clone()).collect();
        for id in ids { self.cancelar(&id)?; }
        Ok(())
    }

    /// Esconde arquivo ou reunião arquivada (reversível com `desfazer`).
    pub fn remover(&self, id: &str) -> Res<()> {
        if self.reuniao.lock().unwrap().as_ref().is_some_and(|a| a.doc == id) { return Err("Encerre a reunião antes de remover.".into()); }
        if self.fila.lock().unwrap().iter().any(|i| i.doc == id) {
            for i in self.fila.lock().unwrap().iter().filter(|i| i.doc == id) { i.cancelado.store(true, Ordering::Release); }
            self.fila.lock().unwrap().retain(|i| i.doc != id || (i.fase != "naFila" && i.fase != "formatoNaoSuportado"));
        }
        let leitor = self.leitor();
        if leitor.existe_documento(id)? { leitor.definir(id, CampoDocumento::Oculto(true))?; }
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
        self.leitor().definir(id, CampoDocumento::Nome(nome.to_string()))?;
        self.mudou();
        Ok(())
    }

    /// Nomes iguais (comparação em português, sem acento) unem as duas vozes.
    pub fn renomear_voz(&self, id: &str, voz: &str, nome: &str) -> Res<()> {
        self.leitor().renomear(id, voz, nome)?;
        self.mudou();
        Ok(())
    }

    pub fn reatribuir(&self, id: &str, trecho: &str, voz: Option<&str>, nome: Option<&str>) -> Res<()> {
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
        if self.desligando.load(Ordering::Acquire) { return Err("O Dita.se está fechando.".into()); }
        if atual.is_some() { return Err("Já há uma reunião em andamento.".into()); }
        if self.finalizando.lock().unwrap().is_some() { return Err("Espere a reunião anterior terminar de processar.".into()); }
        let fala = self.motores.fala(false).map_err(|e| format!("Não dá para começar agora: {e}"))?;
        let com_microfone = fonte.ends_with("EMicrofone");
        let doc = uuid();
        self.leitor().criar_documento(&doc, "reuniao", "Reunião", Some(fonte), "gravando")?;
        let relogio = Instant::now();
        let vivo = Arc::new(Vivo { pausada: AtomicBool::new(false), nivel: Mutex::new(0.0), ultimo_audio: Mutex::new(relogio), pausas: Mutex::new((0.0, None)), relogio });

        let mut fontes = vec![(Fonte::Sistema, FonteDeAudio::Mac)];
        if com_microfone { fontes.push((Fonte::Microfone, FonteDeAudio::Mic)); }
        let mut coletor = ColetorDeReuniao::new(doc.clone(), ArmazemLancamento::new(&self.banco)?, self.disco.clone(), 0.0);
        for (_, f) in &fontes { coletor.abrir(*f, 0.0)?; }

        let (envio, recebe) = mpsc::channel::<Captado>();
        let (fim_tx, fim_rx) = mpsc::channel::<()>();
        let mut capturas = Vec::new();
        let mut aviso = None;
        for (fonte_casca, fonte_nucleo) in &fontes {
            let (tx, v, f) = (envio.clone(), vivo.clone(), *fonte_nucleo);
            let ao_bloco: AoBloco = Box::new(move |t, bloco, nivel| {
                if v.pausada.load(Ordering::Acquire) { return; }
                { let mut n = v.nivel.lock().unwrap(); *n = n.max(nivel); }
                if nivel > 0.05 { *v.ultimo_audio.lock().unwrap() = Instant::now(); }
                let _ = tx.send(Captado::Audio(f, t, bloco));
            });
            match self.capturador.abrir(*fonte_casca, relogio, ao_bloco) {
                Ok(g) => capturas.push(g),
                Err(e) => {
                    nota(format!("reunião: fonte {} indisponível ({e})", fonte_nucleo.nome()));
                    aviso = Some(if *fonte_casca == Fonte::Microfone { "O microfone não abriu; só o áudio do computador está sendo gravado." } else { "O áudio do computador não abriu; só o microfone está sendo gravado." });
                }
            }
        }
        if capturas.is_empty() {
            let mut leitor = self.leitor();
            let _ = leitor.descartar(&doc);
            return Err("Nenhuma fonte de áudio abriu. Confira o microfone e a saída de som.".into());
        }
        if let Some(a) = aviso { self.leitor().definir(&doc, CampoDocumento::AvisoCaptura(Some(a.into())))?; }

        // Coletor: grava cada bloco em disco antes de qualquer reconhecimento.
        let servico = Arc::downgrade(self);
        let doc_c = doc.clone();
        thread::Builder::new().name("ditase.coletor".into()).spawn(move || {
            let mut falha = None;
            for m in recebe {
                let r = match m {
                    Captado::Audio(f, t, a) => coletor.receber(f, t, &a),
                    Captado::Pausar(t) => coletor.pausar(t),
                    Captado::Retomar(t) => { coletor.retomar(t); Ok(()) }
                    Captado::Encerrar(t) => { let r = coletor.encerrar(t); if let Err(e) = r { falha = Some(e.to_string()); } break; }
                };
                if let Err(e) = r { falha = Some(e.to_string()); break; }
            }
            if let Some(e) = falha {
                nota(format!("reunião {doc_c}: gravação em disco falhou ({e})"));
                let _ = coletor.armazem.definir(&doc_c, CampoDocumento::ErroCaptura(Some("A gravação em disco falhou. O áudio já gravado pode ser recuperado.".into())));
                if let Some(s) = servico.upgrade() { s.parar_capturas(&doc_c); }
            }
            let _ = fim_tx.send(());
        })?;

        // Processador: trechos ao vivo a cada segundo; no fim, passada final e resumo.
        let servico = self.clone();
        let (doc_p, vivo_p) = (doc.clone(), vivo.clone());
        thread::Builder::new().name("ditase.reuniao".into()).spawn(move || servico.processar_reuniao(&doc_p, fala, Some((vivo_p, fim_rx))))?;

        *atual = Some(Ativa { doc: doc.clone(), vivo, capturas, envio });
        drop(atual);
        nota(format!("reunião {doc} iniciada ({})", if com_microfone { "sistema e microfone" } else { "sistema" }));
        self.mudou();
        Ok(doc)
    }

    /// Fecha as fontes (falha de disco no coletor): a reunião segue para o fim com o que foi gravado.
    fn parar_capturas(&self, doc: &str) {
        let ativa = { let mut r = self.reuniao.lock().unwrap(); if r.as_ref().is_some_and(|a| a.doc == doc) { r.take() } else { None } };
        if let Some(a) = ativa {
            *self.finalizando.lock().unwrap() = Some(a.doc.clone());
            drop(a.capturas);
            let _ = self.leitor().mudar_estado(doc, "processando", &["gravando", "pausada"]);
            self.mudou();
        }
    }

    fn emitir_trechos(&self, doc: &str, novos: &[TrechoNovo], leitura: &ArmazemLancamento, vivo: Option<&Vivo>) {
        if novos.is_empty() { return; }
        let vozes = leitura.documento(doc).ok().flatten().map(|d| serde_json::to_value(d.vozes).unwrap_or_default()).unwrap_or_else(|| json!([]));
        let duracao = vivo.map_or_else(|| novos.iter().map(|t| t.fim).fold(0.0, f64::max), |v| v.decorrido());
        (self.avisar)("ditado:trechos", Some(json!({"id": doc, "trechos": novos.iter().map(trecho_json).collect::<Vec<_>>(), "vozes": vozes, "duracao": duracao, "atraso": 0})));
    }

    fn processar_reuniao(&self, doc: &str, fala: Arc<dyn crate::integracao::Fala>, ao_vivo: Option<(Arc<Vivo>, mpsc::Receiver<()>)>) {
        let falantes = self.motores.falantes();
        let resultado = (|| -> Res<bool> {
            let mut armazem = ArmazemLancamento::new(&self.banco)?;
            let leitura = ArmazemLancamento::new(&self.banco)?;
            let preparar = self.preparar.clone();
            let preparar = move |t: &str| preparar(t);
            let reconhecedor: &dyn ReconhecedorDeFala = &*fala;
            let mut p = ProcessadorDeReuniao::new(doc, &mut armazem, &self.disco, reconhecedor).com_preparar(&preparar);
            if let Some((vivo, fim)) = &ao_vivo {
                while let Err(mpsc::RecvTimeoutError::Timeout) = fim.recv_timeout(Duration::from_secs(1)) {
                    match p.bloco_selado() {
                        Ok(novos) => self.emitir_trechos(doc, &novos, &leitura, Some(vivo)),
                        Err(e) => nota(format!("reunião {doc}: bloco ao vivo ({e})")),
                    }
                    self.progresso_ao_vivo(doc, vivo);
                }
                let novos = p.fechar()?;
                self.emitir_trechos(doc, &novos, &leitura, Some(vivo));
            } else {
                let novos = p.recuperar()?;
                self.emitir_trechos(doc, &novos, &leitura, None);
            }
            p.encerrar_costuras()?;
            (self.avisar)("ditado:progresso", Some(json!({"id": doc, "fase": "processando", "etapa": {"id": "passadaFinal", "rotulo": "Passada final"}, "estimativa": false, "longo": false, "reuniaoAtiva": false})));
            let identificados = p.passada_final(falantes.as_deref())?;
            Ok(identificados)
        })();
        let motivo = match &resultado {
            Ok(id) => { nota(format!("reunião {doc}: finalizada, falantes {}", if *id { "identificados" } else { "indisponíveis" })); None }
            Err(e) => { nota(format!("reunião {doc}: processamento incompleto ({e}); áudio mantido para recuperar")); Some("O processamento não terminou. O áudio continua guardado para nova tentativa.".to_string()) }
        };
        if resultado.is_ok() && self.motores.resumo_disponivel() && !self.desligando.load(Ordering::Acquire) { self.gerar_resumo(doc); }
        *self.finalizando.lock().unwrap() = None;
        (self.avisar)("ditado:resultado", Some(json!({"id": doc, "resultado": if motivo.is_some() { "erro" } else { "concluido" }, "motivo": motivo})));
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
        if !pausada && sem_audio >= 3.0 { v["motivo"] = json!("Nenhum som chegando. Confira o volume e o microfone."); }
        (self.avisar)("ditado:progresso", Some(v));
    }

    fn com_ativa<T>(&self, id: &str, f: impl FnOnce(&Ativa) -> T) -> Res<T> {
        let r = self.reuniao.lock().unwrap();
        match r.as_ref() { Some(a) if a.doc == id => Ok(f(a)), _ => Err("Essa reunião não está em andamento.".into()) }
    }

    /// Alterna pausa e gravação. O tempo pausado fica fora da transcrição.
    pub fn pausar(&self, id: &str) -> Res<()> {
        let pausou = self.com_ativa(id, |a| {
            let t = a.vivo.relogio.elapsed().as_secs_f64();
            let pausada = !a.vivo.pausada.load(Ordering::Acquire);
            let mut pausas = a.vivo.pausas.lock().unwrap();
            if pausada { pausas.1 = Some(t); } else if let Some(d) = pausas.1.take() { pausas.0 += t - d; }
            a.vivo.pausada.store(pausada, Ordering::Release);
            let _ = a.envio.send(if pausada { Captado::Pausar(t) } else { Captado::Retomar(t) });
            pausada
        })?;
        let leitor = self.leitor();
        if pausou { leitor.mudar_estado(id, "pausada", &["gravando"])?; } else { leitor.mudar_estado(id, "gravando", &["pausada"])?; }
        drop(leitor);
        self.mudou();
        Ok(())
    }

    /// Encerra a captura; a passada final e o resumo seguem em segundo plano.
    pub fn encerrar(&self, id: &str) -> Res<()> {
        let ativa = { let mut r = self.reuniao.lock().unwrap(); match r.as_ref() { Some(a) if a.doc == id => r.take(), _ => None } };
        let a = ativa.ok_or("Essa reunião não está em andamento.")?;
        *self.finalizando.lock().unwrap() = Some(a.doc.clone());
        let t = a.vivo.relogio.elapsed().as_secs_f64();
        drop(a.capturas);
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
        if self.reuniao_atual().as_deref() == Some(id) { return Err("Essa reunião já está em processamento.".into()); }
        {
            let mut f = self.finalizando.lock().unwrap();
            if f.is_some() { return Err("Espere a reunião anterior terminar de processar.".into()); }
            *f = Some(id.to_string());
        }
        let fala = match self.motores.fala(false) { Ok(f) => f, Err(e) => { *self.finalizando.lock().unwrap() = None; return Err(e.into()); } };
        let _ = self.leitor().mudar_estado(id, "processando", &["gravando", "pausada", "interrompida"]);
        let s = self.clone();
        let doc = id.to_string();
        thread::Builder::new().name("ditase.recuperar".into()).spawn(move || s.processar_reuniao(&doc, fala, None))?;
        self.mudou();
        Ok(())
    }

    pub fn descartar_reuniao(&self, id: &str) -> Res<()> {
        if self.reuniao_atual().as_deref() == Some(id) { return Err("Essa reunião está em andamento.".into()); }
        let mut leitor = self.leitor();
        leitor.descartar(id)?;
        leitor.limpar(id, &self.disco)?;
        drop(leitor);
        self.disco.remover_pasta(id);
        self.mudou();
        Ok(())
    }

    // MARK: Resumo

    fn gerar_resumo(&self, doc: &str) {
        self.resumo_cancelado.store(false, Ordering::Release);
        let Ok(Some(d)) = ArmazemLancamento::new(&self.banco).and_then(|a| a.documento(doc)) else { return };
        if d.trechos.is_empty() { return; }
        let nomes: HashMap<&str, &str> = d.vozes.iter().map(|v| (v.id.as_str(), v.nome.as_str())).collect();
        let linhas: Vec<String> = d.trechos.iter().map(|t| format!("{}: {}", nomes.get(t.voz.as_str()).copied().unwrap_or(&t.voz), t.texto)).collect();
        (self.avisar)("ditado:progresso", Some(json!({"id": doc, "fase": "processando", "etapa": {"id": "resumo", "rotulo": "Gerando resumo"}, "estimativa": false, "longo": false, "reuniaoAtiva": false})));
        let cancelado = self.resumo_cancelado.clone();
        let r = self.motores.resumir(&linhas, &|| cancelado.load(Ordering::Acquire));
        let Ok(armazem) = ArmazemLancamento::new(&self.banco) else { return };
        let campo = match r {
            _ if self.resumo_cancelado.load(Ordering::Acquire) => CampoDocumento::ResumoCancelado,
            Ok(topicos) if !topicos.is_empty() => CampoDocumento::Resumo(topicos),
            Ok(_) => CampoDocumento::ErroResumo("O resumo veio vazio.".into()),
            Err(e) => { nota(format!("reunião {doc}: resumo falhou")); CampoDocumento::ErroResumo(e) }
        };
        if let Err(e) = armazem.definir(doc, campo) { nota(format!("reunião {doc}: resumo não gravado ({e})")); }
    }

    /// Gera de novo; a reunião aparece como "finalizando" e o resumo anterior fica até sair o novo.
    pub fn resumir(self: &Arc<Self>, id: &str) -> Res<()> {
        if !self.motores.resumo_disponivel() { return Err("Ligue a IA local nos Ajustes e baixe o modelo para gerar o resumo.".into()); }
        {
            let mut f = self.finalizando.lock().unwrap();
            if f.is_some() { return Err("Espere o processamento atual terminar.".into()); }
            *f = Some(id.to_string());
        }
        let s = self.clone();
        let doc = id.to_string();
        thread::Builder::new().name("ditase.resumo".into()).spawn(move || {
            s.gerar_resumo(&doc);
            *s.finalizando.lock().unwrap() = None;
            s.mudou();
        })?;
        self.mudou();
        Ok(())
    }

    pub fn cancelar_resumo(&self, id: &str) {
        if self.finalizando.lock().unwrap().as_deref() == Some(id) { self.resumo_cancelado.store(true, Ordering::Release); }
    }

    /// Fecha as fontes e espera, até `prazo`, a reunião em curso e a fila terminarem.
    ///
    /// A reunião em andamento guarda o canal do coletor dentro do serviço, e a thread de
    /// processamento guarda o serviço: sem isto, soltar o serviço deixava coletor e processador
    /// vivos para sempre, com as conexões SQLite e a pasta de áudio abertas. O que não terminar no
    /// prazo continua em disco e volta em `recuperaveis` na próxima abertura.
    pub fn desligar(&self, prazo: Duration) {
        self.desligando.store(true, Ordering::Release);
        self.resumo_cancelado.store(true, Ordering::Release);
        let ativa = self.reuniao.lock().unwrap().take();
        if let Some(a) = ativa {
            *self.finalizando.lock().unwrap() = Some(a.doc.clone());
            let t = a.vivo.relogio.elapsed().as_secs_f64();
            drop(a.capturas);
            let _ = a.envio.send(Captado::Encerrar(t));
        }
        self.cancelar_fila().ok();
        let limite = Instant::now() + prazo;
        while (self.finalizando.lock().unwrap().is_some() || !self.fila.lock().unwrap().is_empty()) && Instant::now() < limite {
            thread::sleep(Duration::from_millis(20));
        }
    }
}

#[cfg(test)]
pub mod dubles {
    use std::{sync::{atomic::{AtomicBool, Ordering}, Arc}, thread, time::Instant};

    use super::{AoBloco, Capturador};
    use crate::{audio::Fonte, integracao::dubles::voz};

    /// Dublê de captura: entrega `segundos` de voz em blocos de 100 ms, com o tempo pela contagem de amostras.
    pub struct CapturaDuble { pub segundos: f64 }
    impl Capturador for CapturaDuble {
        fn abrir(&self, fonte: Fonte, _relogio: Instant, mut ao_bloco: AoBloco) -> Result<Box<dyn Send>, String> {
            let pcm = if fonte == Fonte::Sistema { voz(self.segundos, 0.6) } else { voz(1.0, self.segundos - 0.4) };
            let fim = Arc::new(AtomicBool::new(false));
            let parar = fim.clone();
            let fio = thread::spawn(move || {
                for (i, bloco) in pcm.chunks(1600).enumerate() {
                    if parar.load(Ordering::Acquire) { break; }
                    ao_bloco(i as f64 * 0.1, bloco.to_vec(), 0.3);
                }
            });
            struct Guarda(Arc<AtomicBool>, Option<thread::JoinHandle<()>>);
            impl Drop for Guarda { fn drop(&mut self) { self.0.store(true, Ordering::Release); if let Some(f) = self.1.take() { let _ = f.join(); } } }
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
        let tmp = tempfile::tempdir().unwrap();
        let reg = Arc::new(Registro(Mutex::new(Vec::new())));
        let r = reg.clone();
        let avisar: Avisar = Arc::new(move |a, v| r.0.lock().unwrap().push((a.to_string(), v)));
        let preparar: Preparar = Arc::new(|t: &str| nucleo::RegrasFala::filtrar(t));
        let s = Lancamento::new(&tmp.path().join("t.sqlite"), &tmp.path().join("Audio"), MotoresDuble::novo(), Arc::new(CapturaDuble { segundos }), preparar, avisar).unwrap();
        (tmp, s, reg)
    }

    fn esperar(mut condicao: impl FnMut() -> bool, o_que: &str) {
        let inicio = Instant::now();
        while !condicao() {
            assert!(inicio.elapsed() < Duration::from_secs(30), "tempo esgotado esperando {o_que}");
            thread::sleep(Duration::from_millis(50));
        }
    }

    fn wav(pasta: &Path, nome: &str, pcm: &[f32]) -> PathBuf {
        let p = pasta.join(nome);
        let mut w = crate::audio::EscritorWav::criar(&p).unwrap();
        w.gravar(pcm).unwrap();
        p
    }

    #[test]
    fn arquivo_passa_pelo_processador_com_falantes_e_fica_pronto() {
        let (tmp, s, reg) = servico(3.0);
        assert_eq!(s.estado(true).unwrap()["disponivel"], true);
        let arq = wav(tmp.path(), "pauta.wav", &voz(3.0, 1.0));
        let id = s.adicionar(&[arq]).unwrap().unwrap();
        esperar(|| s.estado(true).unwrap()["arquivos"][0]["fase"] == "pronto", "arquivo pronto");
        let doc = s.documento(&id).unwrap();
        assert_eq!(doc["nome"], "pauta.wav");
        assert!(!doc["trechos"].as_array().unwrap().is_empty());
        assert!(doc["vozes"].as_array().unwrap().iter().any(|v| v["id"] == "offline-1"), "{doc}");
        let lista = s.estado(true).unwrap();
        assert!(lista["arquivos"][0]["palavras"].as_u64().unwrap() > 0);
        assert!(reg.0.lock().unwrap().iter().any(|(a, v)| a == "ditado:resultado" && v.as_ref().unwrap()["resultado"] == "concluido"));
        let progresso: Vec<f64> = reg.0.lock().unwrap().iter().filter(|(a, _)| a == "ditado:progresso").filter_map(|(_, v)| v.as_ref()?["progresso"].as_f64()).collect();
        assert!(progresso.windows(2).all(|p| p[1] >= p[0]), "progresso voltou: {progresso:?}");
        // Busca, título, falante, remover e desfazer.
        assert_eq!(s.procurar_arquivos("PALAVRA").unwrap()[0], json!(id));
        s.alterar_titulo(&id, "Pauta revisada").unwrap();
        s.renomear_voz(&id, "offline-1", "Ana").unwrap();
        let doc = s.documento(&id).unwrap();
        assert_eq!(doc["nome"], "Pauta revisada");
        assert!(doc["vozes"].as_array().unwrap().iter().any(|v| v["nome"] == "Ana"));
        s.remover(&id).unwrap();
        assert!(s.estado(true).unwrap()["arquivos"].as_array().unwrap().is_empty());
        s.desfazer(&id).unwrap();
        assert_eq!(s.estado(true).unwrap()["arquivos"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn formato_nao_suportado_e_arquivo_ilegivel() {
        let (tmp, s, _) = servico(3.0);
        let txt = tmp.path().join("notas.txt");
        std::fs::write(&txt, "x").unwrap();
        s.adicionar(&[txt]).unwrap();
        assert_eq!(s.estado(true).unwrap()["arquivos"][0]["fase"], "formatoNaoSuportado");
        let ruim = tmp.path().join("quebrado.mp3");
        std::fs::write(&ruim, b"isto nao e mp3").unwrap();
        let id = s.adicionar(&[ruim]).unwrap().unwrap();
        esperar(|| s.estado(true).unwrap()["arquivos"].as_array().unwrap().iter().any(|a| a["id"] == json!(id) && a["fase"] == "falhou"), "falha registrada");
    }

    #[test]
    fn reuniao_com_microfone_e_sistema_grava_transcreve_e_identifica_falantes() {
        let (_tmp, s, reg) = servico(4.0);
        let id = s.iniciar_reuniao("macEMicrofone").unwrap();
        assert!(s.iniciar_reuniao("mac").is_err(), "segunda reunião ao mesmo tempo");
        assert_eq!(s.estado(true).unwrap()["reuniao"]["fase"], "gravando");
        thread::sleep(Duration::from_millis(1500));
        s.encerrar(&id).unwrap();
        esperar(|| s.estado(true).unwrap()["anteriores"].as_array().unwrap().iter().any(|r| r["id"] == json!(id)), "reunião arquivada");
        let doc = s.documento(&id).unwrap();
        assert_eq!(doc["fase"], "pronta");
        assert_eq!(doc["falantes"], "identificados");
        let trechos = doc["trechos"].as_array().unwrap();
        assert!(!trechos.is_empty());
        assert!(trechos.iter().all(|t| t["parcial"] != true));
        assert!(s.estado(true).unwrap()["recuperaveis"].as_array().unwrap().is_empty(), "áudio liberado depois da passada final");
        let eventos = reg.0.lock().unwrap();
        assert!(eventos.iter().any(|(a, _)| a == "ditado:trechos"), "trechos ao vivo");
        assert!(eventos.iter().any(|(a, v)| a == "ditado:resultado" && v.as_ref().unwrap()["resultado"] == "concluido"));
    }

    #[test]
    fn reuniao_interrompida_fica_recuperavel_e_recupera_sem_perder_fala() {
        let tmp = tempfile::tempdir().unwrap();
        let banco = tmp.path().join("t.sqlite");
        let audio = tmp.path().join("Audio");
        let id;
        {
            // Captura direta pelo coletor, sem processar: simula o app fechando no meio.
            let armazem = ArmazemLancamento::new(&banco).unwrap();
            id = uuid();
            armazem.criar_documento(&id, "reuniao", "Reunião", Some("mac"), "gravando").unwrap();
            let mut c = ColetorDeReuniao::new(id.clone(), ArmazemLancamento::new(&banco).unwrap(), DiscoDeBlocos::new(audio.clone()), 0.0);
            c.abrir(FonteDeAudio::Mac, 0.0).unwrap();
            for (i, b) in voz(3.0, 0.5).chunks(1600).enumerate() { c.receber(FonteDeAudio::Mac, i as f64 * 0.1, b).unwrap(); }
            c.selar_tudo().unwrap();
        }
        let avisar: Avisar = Arc::new(|_, _| {});
        let s = Lancamento::new(&banco, &audio, MotoresDuble::novo(), Arc::new(CapturaDuble { segundos: 1.0 }), Arc::new(|t: &str| t.to_string()), avisar).unwrap();
        let rec = s.estado(true).unwrap()["recuperaveis"].clone();
        assert!(rec.as_array().unwrap().iter().any(|r| r["id"] == json!(id)), "{rec}");
        s.recuperar_reuniao(&id).unwrap();
        esperar(|| s.documento(&id).unwrap()["fase"] == "pronta", "recuperação");
        let doc = s.documento(&id).unwrap();
        let palavras: usize = doc["trechos"].as_array().unwrap().iter().map(|t| t["texto"].as_str().unwrap().split_whitespace().count()).sum();
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
