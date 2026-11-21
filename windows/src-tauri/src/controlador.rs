//! Ciclo do ditado (porta de `Controlador.swift`): atalho, microfone, sessão, banco, colagem e
//! pílula. A sessão (Parakeet, regras, dicionário e limpeza opcional) e o banco vêm de
//! [`crate::integracao`]. Sem modelo de fala pronto, o ditado não começa e a pílula diz por quê.
//!
//! O que é do Windows e da janela entra por traits ([`Avisos`] para a pílula, [`Sistema`] para
//! microfone, app em foco e colagem), para o ciclo inteiro ser testado sem janela.
use std::{
    path::PathBuf,
    sync::{mpsc, Arc, Mutex, OnceLock, Weak},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use crate::{
    atalho::{Acoes, Atalho, Entrada},
    audio::{self, Captura, EscritorWav, Fonte},
    caminhos::{self, nota},
    emenda::{Destino, Emenda, FimDaDerivacao},
    integracao::{Ditado, Integracao, SessaoDeDitado},
    nativo::{self, Insercao},
    pilula::{Fase as FasePilula, Pilula},
    preferencias::Preferencias,
};

/// O que o ditado mostra na pílula.
pub trait Avisos: Send + Sync {
    fn mostrar(&self, fase: FasePilula);
    fn avisar(&self, texto: &str, segundos: f64);
    fn nivel(&self, nivel: f32);
    fn segundos(&self, s: u64);
    fn atalho(&self, simbolo: &str);
    fn sempre_visivel(&self, sim: bool);
}

impl Avisos for Pilula {
    fn mostrar(&self, fase: FasePilula) {
        Pilula::mostrar(self, fase)
    }
    fn avisar(&self, texto: &str, segundos: f64) {
        Pilula::avisar(self, texto, segundos)
    }
    fn nivel(&self, nivel: f32) {
        self.empurrar_nivel(nivel)
    }
    fn segundos(&self, s: u64) {
        Pilula::segundos(self, s)
    }
    fn atalho(&self, simbolo: &str) {
        Pilula::atalho(self, simbolo)
    }
    fn sempre_visivel(&self, sim: bool) {
        Pilula::sempre_visivel(self, sim)
    }
}

/// Fonte de áudio aberta para o ditado. `parar` fecha e entrega o que faltava.
pub trait FonteAberta: Send {
    fn parar(&mut self);
}
impl FonteAberta for Captura {
    fn parar(&mut self) {
        Captura::parar(self)
    }
}

/// Recebe PCM 16 kHz mono e o nível do bloco.
pub type AoBlocoDeAudio = Box<dyn FnMut(Vec<f32>, f32) + Send>;
/// Aviso de que a reunião vai soltar o microfone com o ditado ainda derivando. Devolve o que fazer
/// depois que ela soltar (segunda tentativa de abrir o microfone próprio), se houver.
pub type AoAcabar = Box<dyn FnOnce() -> Option<DepoisDeSoltar> + Send>;
/// Segunda fase do aviso: roda depois que a reunião fechou a captura e a derivação.
pub type DepoisDeSoltar = Box<dyn FnOnce() + Send>;

/// Quanto o ditado espera o primeiro bloco do microfone próprio, com a derivação ainda viva, antes
/// de soltar a derivação (N-1). Sem bloco nesse prazo, troca assim mesmo e registra no log.
const SOBREPOSICAO: Duration = Duration::from_millis(500);

/// Windows: microfone, app em foco, colagem e a pasta do áudio guardado.
pub trait Sistema: Send + Sync {
    fn microfone_disponivel(&self) -> bool;
    fn abrir_microfone(
        &self,
        nome: &str,
        ao_bloco: AoBlocoDeAudio,
    ) -> Result<Box<dyn FonteAberta>, String>;
    fn app_em_foco(&self) -> (String, String);
    fn colar(&self, texto: &str) -> Insercao;
    fn pasta_de_audio(&self) -> PathBuf;
}

pub struct SistemaReal;

impl Sistema for SistemaReal {
    fn microfone_disponivel(&self) -> bool {
        audio::microfone_disponivel()
    }
    fn abrir_microfone(
        &self,
        nome: &str,
        ao_bloco: AoBlocoDeAudio,
    ) -> Result<Box<dyn FonteAberta>, String> {
        Captura::iniciar(Fonte::Microfone, nome, ao_bloco)
            .map(|c| Box::new(c) as Box<dyn FonteAberta>)
            .map_err(|e| e.to_string())
    }
    fn app_em_foco(&self) -> (String, String) {
        nativo::sistema::app_em_foco()
    }
    fn colar(&self, texto: &str) -> Insercao {
        nativo::colagem::inserir(texto)
    }
    fn pasta_de_audio(&self) -> PathBuf {
        caminhos::audios()
    }
}

/// Microfone da reunião em andamento (I5). Com reunião gravando o microfone, o ditado usa uma
/// derivação desse PCM em vez de abrir uma segunda captura do mesmo dispositivo (headsets Bluetooth
/// e drivers exclusivos recusam ou derrubam a segunda abertura).
pub trait MicrofoneCompartilhado: Send + Sync {
    /// Derivação nova, ou `None` sem reunião com microfone. Soltar a fonte devolvida encerra a derivação.
    /// `ao_acabar` é chamado quando a reunião vai soltar o microfone com o ditado ainda derivando,
    /// com a captura da reunião ainda aberta e a derivação ainda entregando blocos: o ditado abre o
    /// microfone próprio e emenda as duas fontes pelo relógio (N-1). Depois que `ao_acabar` volta,
    /// a reunião fecha a captura, solta o callback da derivação e, se recebeu um [`DepoisDeSoltar`]
    /// (o microfone próprio não abriu com o dispositivo ocupado), chama-o sem nenhuma trava.
    fn derivar(
        &self,
        ao_bloco: AoBlocoDeAudio,
        ao_acabar: AoAcabar,
    ) -> Option<Box<dyn FonteAberta>>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fase {
    Parado,
    Gravando,
    Processando,
}

impl Fase {
    pub fn id(self) -> &'static str {
        match self {
            Fase::Parado => "parado",
            Fase::Gravando => "gravando",
            Fase::Processando => "processando",
        }
    }
}

struct Gravacao {
    ditado: Ditado,
    inicio: Instant,
    captura: Box<dyn FonteAberta>,
    destino: Option<Destino>,
    bomba: thread::JoinHandle<(Box<dyn SessaoDeDitado>, usize)>,
    silencio: Arc<Mutex<f64>>,
}

struct Interno {
    fase: Fase,
    maos_livres: bool,
    gravacao: Option<Gravacao>,
}

pub struct Controlador {
    pub prefs: Arc<Preferencias>,
    pub integracao: Arc<Integracao>,
    pilula: Arc<dyn Avisos>,
    sistema: Arc<dyn Sistema>,
    compartilhado: OnceLock<Arc<dyn MicrofoneCompartilhado>>,
    pub atalho: OnceLock<Atalho>,
    pub atalho_instalado: Mutex<bool>,
    interno: Mutex<Interno>,
    /// Avisa a interface: "estado", "historico" ou "dicionario".
    ao_mudar: Box<dyn Fn(&str) + Send + Sync>,
}

impl Controlador {
    pub fn new(
        prefs: Arc<Preferencias>,
        integracao: Arc<Integracao>,
        pilula: Arc<dyn Avisos>,
        sistema: Arc<dyn Sistema>,
        ao_mudar: impl Fn(&str) + Send + Sync + 'static,
    ) -> Arc<Self> {
        Arc::new(Self {
            prefs,
            integracao,
            pilula,
            sistema,
            compartilhado: OnceLock::new(),
            atalho: OnceLock::new(),
            atalho_instalado: Mutex::new(false),
            interno: Mutex::new(Interno {
                fase: Fase::Parado,
                maos_livres: false,
                gravacao: None,
            }),
            ao_mudar: Box::new(ao_mudar),
        })
    }

    /// Microfone da reunião, para o ditado durante a reunião (ligado depois de criar o serviço de reuniões).
    pub fn compartilhar_microfone(&self, m: Arc<dyn MicrofoneCompartilhado>) {
        let _ = self.compartilhado.set(m);
    }

    /// Recupera ditados interrompidos e apaga áudio vencido. Fora do caminho da janela (thread).
    pub fn manutencao(&self) {
        match self
            .integracao
            .recuperar_interrompidos(&self.sistema.pasta_de_audio())
        {
            Ok(n) if n > 0 => nota(format!(
                "recuperação: {n} ditado(s) interrompido(s) voltaram ao histórico"
            )),
            Err(e) => nota(format!("recuperação: {e}")),
            _ => {}
        }
        self.limpar_audios_antigos();
    }

    /// Liga atalho, manutenção e relógio. Chamado uma vez na abertura.
    pub fn ligar(self: &Arc<Self>) {
        let c = self.clone();
        thread::Builder::new()
            .name("ditase.manutencao".into())
            .spawn(move || c.manutencao())
            .expect("manutenção");
        let atalho = Atalho::iniciar(self.prefs.atalho().vk(), Ponte(Arc::downgrade(self)));
        let instalado =
            nativo::gancho::instalar(atalho.visao.clone(), atalho.emissor(), atalho.relogio);
        *self.atalho_instalado.lock().unwrap() = instalado;
        let _ = self.atalho.set(atalho);
        let fraco = Arc::downgrade(self);
        thread::Builder::new()
            .name("ditase.relogio".into())
            .spawn(move || loop {
                thread::sleep(Duration::from_millis(500));
                let Some(c) = fraco.upgrade() else { break };
                c.acompanhar();
            })
            .expect("relógio");
    }

    pub fn fase(&self) -> Fase {
        self.interno.lock().unwrap().fase
    }
    pub fn maos_livres(&self) -> bool {
        self.interno.lock().unwrap().maos_livres
    }
    fn mudou(&self, assunto: &str) {
        (self.ao_mudar)(assunto)
    }
    fn avisar_atalho(&self, e: Entrada) {
        if let Some(a) = self.atalho.get() {
            a.enviar(e);
        }
    }

    pub fn preferencia_mudou(&self, chave: &str) {
        match chave {
            "atalho" => {
                let t = self.prefs.atalho();
                if let Some(a) = self.atalho.get() {
                    a.trocar_tecla(t.vk());
                }
                self.pilula.atalho(t.simbolo());
            }
            "pilulaSempreVisivel" => self
                .pilula
                .sempre_visivel(self.prefs.pilula_sempre_visivel()),
            _ => {}
        }
        self.mudou("estado");
    }

    // MARK: Ciclo do ditado

    pub fn comecar(self: &Arc<Self>, maos_livres: bool) -> bool {
        let mut i = self.interno.lock().unwrap();
        if i.fase != Fase::Parado {
            return false;
        }
        if !self.sistema.microfone_disponivel() {
            drop(i);
            self.pilula.avisar(
                "Microfone indisponível. Confira Privacidade > Microfone",
                3.5,
            );
            return false;
        }
        let (app, app_nome) = self.sistema.app_em_foco();
        let mut d = Ditado {
            iniciado_em: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|x| x.as_secs_f64())
                .unwrap_or(0.0),
            app,
            app_nome,
            estado: "gravando".into(),
            motor: "Parakeet".into(),
            ..Default::default()
        };
        d.id = match self.integracao.inserir_ditado(&d) {
            Ok(id) => id,
            Err(e) => {
                nota(format!("banco: {e}"));
                drop(i);
                self.pilula.avisar("Erro ao abrir o histórico", 3.0);
                return false;
            }
        };
        let limpeza = self.prefs.limpeza_ia();
        let sessao = match self.integracao.nova_sessao(d.id, limpeza) {
            Ok(s) => s,
            Err(motivo) => {
                let _ = self.integracao.descartar_ditado(d.id);
                drop(i);
                nota(format!("ditado recusado: {motivo}"));
                self.pilula.avisar(&motivo, 3.5);
                return false;
            }
        };
        if limpeza {
            self.integracao.motores.aquecer();
        }
        let caminho = (self.prefs.guardar_audio_dias() > 0)
            .then(|| self.sistema.pasta_de_audio().join(format!("{}.wav", d.id)));
        let mut escritor = caminho.as_ref().and_then(|c| {
            EscritorWav::criar(c)
                .map_err(|e| nota(format!("wav: {e}")))
                .ok()
        });
        if escritor.is_some() {
            d.audio = caminho
                .as_ref()
                .and_then(|c| c.file_name())
                .map(|n| n.to_string_lossy().into())
                .unwrap_or_default();
            if let Err(e) = self.integracao.definir_audio(d.id, &d.audio) {
                nota(format!("ditado {}: áudio não registrado ({e})", d.id));
            }
        }

        let (tx, rx) = mpsc::channel::<Vec<f32>>();
        let pilula = self.pilula.clone();
        let ao_bloco: AoBlocoDeAudio = Box::new(move |bloco, nivel| {
            pilula.nivel(nivel);
            let _ = tx.send(bloco);
        });
        // Reunião gravando o microfone: deriva dela (I5). Senão, abre o microfone.
        let (ao_bloco, derivada, destino) = match self.compartilhado.get() {
            Some(m) => {
                let parte: Destino = Arc::new(Mutex::new(Some(ao_bloco)));
                let emenda = Arc::new(Mutex::new(Emenda::new(parte.clone(), Instant::now())));
                let (fraco, id, e2) = (Arc::downgrade(self), d.id, emenda.clone());
                let ao_acabar: AoAcabar =
                    Box::new(move || fraco.upgrade().and_then(|c| c.reuniao_acabou(id, e2)));
                // Soltar este callback (reunião acabou ou o ditado trocou de fonte) fecha a derivação.
                let fim = FimDaDerivacao(emenda);
                match m.derivar(
                    Box::new(move |b, n| fim.0.lock().unwrap().derivado(b, n, Instant::now())),
                    ao_acabar,
                ) {
                    Some(f) => (None, Some(f), Some(parte)),
                    None => (parte.lock().unwrap().take(), None, None),
                }
            }
            None => (Some(ao_bloco), None, None),
        };
        let aberta = match (derivada, ao_bloco) {
            (Some(f), _) => {
                nota(format!("ditado {}: microfone derivado da reunião", d.id));
                Ok(f)
            }
            (None, Some(f)) => self.sistema.abrir_microfone(&self.prefs.microfone(), f),
            (None, None) => Err("microfone indisponível".into()),
        };
        let captura = match aberta {
            Ok(c) => c,
            Err(e) => {
                nota(format!("microfone: {e}"));
                let _ = self.integracao.descartar_ditado(d.id);
                // O WAV de 44 bytes criado acima não fica órfão em Audio/.
                drop(escritor);
                if let Some(c) = &caminho {
                    let _ = std::fs::remove_file(c);
                }
                drop(i);
                self.pilula.avisar("Microfone indisponível", 3.0);
                return false;
            }
        };
        let silencio = Arc::new(Mutex::new(0.0f64));
        let silencio_bomba = silencio.clone();
        let bomba = thread::Builder::new()
            .name("ditase.bomba".into())
            .spawn(move || {
                let mut sessao = sessao;
                let mut total = 0usize;
                for bloco in rx {
                    total += bloco.len();
                    if let Some(w) = escritor.as_mut() {
                        if let Err(e) = w.gravar(&bloco) {
                            nota(format!("wav: {e}"));
                            escritor = None;
                        }
                    }
                    sessao.alimentar(&bloco);
                    *silencio_bomba.lock().unwrap() = sessao.silencio();
                }
                (sessao, total)
            })
            .expect("bomba");

        let id = d.id;
        i.gravacao = Some(Gravacao {
            ditado: d,
            inicio: Instant::now(),
            captura,
            destino,
            bomba,
            silencio,
        });
        i.fase = Fase::Gravando;
        i.maos_livres = maos_livres;
        drop(i);
        // Pequena espera: se a tecla foi usada como modificador, a pílula nem aparece.
        let fraco = Arc::downgrade(self);
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(if maos_livres { 0 } else { 120 }));
            let Some(c) = fraco.upgrade() else { return };
            let i = c.interno.lock().unwrap();
            if i.fase == Fase::Gravando && i.gravacao.as_ref().map(|g| g.ditado.id) == Some(id) {
                let fase = if i.maos_livres {
                    FasePilula::MaosLivres
                } else {
                    FasePilula::Gravando
                };
                drop(i);
                c.pilula.mostrar(fase);
            }
        });
        self.mudou("estado");
        true
    }

    fn gravando_ditado(&self, id: i64) -> bool {
        let i = self.interno.lock().unwrap();
        i.fase == Fase::Gravando && i.gravacao.as_ref().is_some_and(|g| g.ditado.id == id)
    }

    /// A reunião vai soltar o microfone com este ditado derivando dele (I-2, N-1). Chamado com a
    /// captura da reunião ainda aberta: abre o microfone próprio, mantém as duas fontes até o
    /// primeiro bloco dele e só então solta a derivação; a [`Emenda`] corta pelo relógio. Se o
    /// microfone não abrir agora (dispositivo exclusivo ocupado pela reunião), devolve a segunda
    /// tentativa para depois que a reunião soltar. Nunca grava silêncio.
    fn reuniao_acabou(
        self: &Arc<Self>,
        id: i64,
        emenda: Arc<Mutex<Emenda>>,
    ) -> Option<DepoisDeSoltar> {
        if !self.gravando_ditado(id) {
            return None;
        }
        match self.trocar_para_proprio(id, &emenda) {
            Ok(()) => None,
            Err(e) => {
                nota(format!(
                    "ditado {id}: microfone próprio não abriu com a reunião aberta ({e}); tento depois que ela soltar"
                ));
                let c = self.clone();
                Some(Box::new(move || c.depois_de_soltar(id, emenda)))
            }
        }
    }

    /// Segunda tentativa, com o microfone já solto pela reunião. A fala dita enquanto ele abre se
    /// perde: a pílula avisa. Se nem assim abrir, encerra com o que já tem e avisa.
    fn depois_de_soltar(self: &Arc<Self>, id: i64, emenda: Arc<Mutex<Emenda>>) {
        if !self.gravando_ditado(id) {
            return;
        }
        match self.trocar_para_proprio(id, &emenda) {
            Ok(()) => self.pilula.avisar(
                "Troquei de microfone: um trecho curto pode ter ficado de fora",
                3.0,
            ),
            Err(e) => {
                nota(format!(
                    "ditado {id}: a reunião acabou e o microfone não abriu ({e}); encerrado"
                ));
                self.avisar_atalho(Entrada::EncerradoPorFora);
                let c = self.clone();
                thread::spawn(move || {
                    c.terminar_com_aviso(
                        true,
                        Some("A reunião acabou e o microfone não abriu. Ditado encerrado com o que tinha"),
                    )
                });
            }
        }
    }

    /// Abre o microfone próprio na emenda, espera o primeiro bloco (até [`SOBREPOSICAO`]) e troca a
    /// fonte do ditado. Soltar a fonte antiga fecha a derivação, que fixa o corte da emenda. Abre
    /// fora da trava do controlador para não bloquear o app.
    fn trocar_para_proprio(
        self: &Arc<Self>,
        id: i64,
        emenda: &Arc<Mutex<Emenda>>,
    ) -> Result<(), String> {
        let e2 = emenda.clone();
        let proprio: AoBlocoDeAudio =
            Box::new(move |b, n| e2.lock().unwrap().proprio(b, n, Instant::now()));
        let mut nova = self
            .sistema
            .abrir_microfone(&self.prefs.microfone(), proprio)?;
        let limite = Instant::now() + SOBREPOSICAO;
        while !emenda.lock().unwrap().proprio_chegou() && Instant::now() < limite {
            thread::sleep(Duration::from_millis(5));
        }
        if !emenda.lock().unwrap().proprio_chegou() {
            nota(format!(
                "ditado {id}: o microfone próprio não entregou áudio em {} ms; troca assim mesmo",
                SOBREPOSICAO.as_millis()
            ));
        }
        let antiga = {
            let mut i = self.interno.lock().unwrap();
            let ainda =
                i.fase == Fase::Gravando && i.gravacao.as_ref().is_some_and(|g| g.ditado.id == id);
            if !ainda {
                // `terminar` levou a gravação enquanto o microfone abria.
                drop(i);
                nova.parar();
                return Ok(());
            }
            let g = i.gravacao.as_mut().expect("conferido acima");
            std::mem::replace(&mut g.captura, nova)
        };
        let mut antiga = antiga;
        antiga.parar();
        nota(format!(
            "ditado {id}: a reunião acabou; segue no microfone próprio"
        ));
        Ok(())
    }

    pub fn entrar_em_maos_livres(&self) {
        let mut i = self.interno.lock().unwrap();
        if i.fase != Fase::Gravando {
            return;
        }
        i.maos_livres = true;
        drop(i);
        self.pilula.mostrar(FasePilula::MaosLivres);
        self.mudou("estado");
    }

    /// Chamado pela interface e pela pílula: começa em mãos livres ou termina o que estiver gravando.
    pub fn alternar(self: &Arc<Self>) {
        match self.fase() {
            Fase::Parado => {
                if self.comecar(true) {
                    self.avisar_atalho(Entrada::IniciadoEmMaosLivres);
                }
            }
            Fase::Gravando => {
                self.avisar_atalho(Entrada::EncerradoPorFora);
                self.terminar(true);
            }
            Fase::Processando => {}
        }
    }

    fn acompanhar(self: &Arc<Self>) {
        let (decorrido, maos_livres, silencio) = {
            let i = self.interno.lock().unwrap();
            if i.fase != Fase::Gravando {
                return;
            }
            let Some(g) = i.gravacao.as_ref() else { return };
            let valores = (
                g.inicio.elapsed().as_secs_f64(),
                i.maos_livres,
                *g.silencio.lock().unwrap(),
            );
            valores
        };
        self.pilula.segundos(decorrido as u64);
        if decorrido >= self.prefs.limite_minutos() * 60.0 {
            nota(format!(
                "ditado encerrado pelo limite de {} min",
                self.prefs.limite_minutos()
            ));
            self.avisar_atalho(Entrada::EncerradoPorFora);
            self.terminar(true);
            return;
        }
        let limite = self.prefs.parar_em_silencio_minutos() * 60.0;
        if maos_livres && limite > 0.0 && silencio >= limite {
            nota(format!(
                "ditado encerrado por {} s de silêncio",
                silencio as u64
            ));
            self.avisar_atalho(Entrada::EncerradoPorFora);
            self.terminar(true);
        }
    }

    pub fn terminar(self: &Arc<Self>, colar: bool) {
        self.terminar_com_aviso(colar, None);
    }

    fn terminar_com_aviso(self: &Arc<Self>, colar: bool, aviso_final: Option<&'static str>) {
        let gravacao = {
            let mut i = self.interno.lock().unwrap();
            if i.fase != Fase::Gravando {
                return;
            }
            i.fase = Fase::Processando;
            i.gravacao.take()
        };
        let Some(g) = gravacao else { return };
        let soltou = Instant::now();
        // A fonte sai já, antes de qualquer mudança para Parado (I-1).
        let Gravacao {
            ditado,
            inicio,
            mut captura,
            destino,
            bomba,
            ..
        } = g;
        captura.parar();
        drop(captura);
        if let Some(d) = destino {
            d.lock().unwrap().take();
        }
        self.pilula.mostrar(FasePilula::Processando);
        self.mudou("estado");
        let c = self.clone();
        thread::spawn(move || {
            let mut d = ditado;
            d.duracao = soltou.duration_since(inicio).as_secs_f64();
            match bomba.join() {
                Ok((sessao, _)) => {
                    let r = sessao.finalizar();
                    d.latencia = soltou.elapsed().as_secs_f64();
                    c.concluir(d, r, colar, aviso_final);
                }
                Err(_) => {
                    nota(format!("ditado {}: a gravação parou com erro", d.id));
                    let _ = c.integracao.marcar_falha(&d);
                    c.pilula.avisar("Não consegui transcrever", 3.0);
                }
            }
            {
                let mut i = c.interno.lock().unwrap();
                i.fase = Fase::Parado;
                i.maos_livres = false;
            }
            c.mudou("estado");
            c.mudou("historico");
        });
    }

    fn concluir(
        &self,
        mut d: Ditado,
        r: crate::integracao::ResultadoDitado,
        colar: bool,
        aviso_final: Option<&'static str>,
    ) {
        if r.texto.is_empty() {
            if r.falhas == 0 {
                // Silêncio de verdade: não há o que guardar.
                let _ = self.integracao.descartar_ditado(d.id);
                apagar_audio(&self.sistema.pasta_de_audio(), &d.audio);
                self.pilula.avisar(
                    aviso_final.unwrap_or("Não ouvi nada"),
                    if aviso_final.is_some() { 3.5 } else { 1.6 },
                );
                return;
            }
            d.estado = "falhou".into();
            if let Err(e) = self.integracao.marcar_falha(&d) {
                nota(format!("ditado {}: não gravou a falha: {e}", d.id));
            }
            self.pilula.avisar(
                if d.audio.is_empty() {
                    "Não consegui transcrever"
                } else {
                    "Não transcrevi. O áudio ficou guardado"
                },
                3.5,
            );
            return;
        }
        d.estado = if r.falhas > 0 { "parcial" } else { "concluido" }.into();
        if let Err(e) = self.integracao.concluir_ditado(&d, &r) {
            nota(format!("ditado {}: resultado não foi gravado: {e}", d.id));
        }
        nota(format!(
            "ditado {}: {} palavras, {:.1} s, espera {:.2} s, limpador {}, recusas {}, falhas {}",
            d.id, r.palavras, d.duracao, d.latencia, r.limpador, r.recusas, r.falhas
        ));
        if !r.trocas.is_empty() {
            self.mudou("dicionario");
        }
        if !colar {
            self.pilula
                .avisar("Ditado guardado no histórico, sem colar", 3.0);
            return;
        }
        let texto = if self.prefs.espaco_no_fim() && !r.texto.ends_with('\n') {
            format!("{} ", r.texto)
        } else {
            r.texto.clone()
        };
        self.colar(&texto, r.falhas > 0, aviso_final);
    }

    fn colar(&self, texto: &str, parcial: bool, aviso_final: Option<&'static str>) {
        match self.sistema.colar(texto) {
            Insercao::Colado => {
                if let Some(aviso) = aviso_final {
                    self.pilula.avisar(aviso, 3.5)
                } else if parcial {
                    self.pilula.avisar("Parte do áudio não foi transcrita", 3.5)
                } else {
                    self.pilula.mostrar(FasePilula::Ociosa)
                }
            }
            other => self.avisar_sem_colar(other),
        }
    }

    fn avisar_sem_colar(&self, i: Insercao) {
        match i {
            Insercao::Colado => {}
            Insercao::SoCopiado => self.pilula.avisar("Copiado. Cole com Ctrl+V", 3.0),
            Insercao::Recusado => self
                .pilula
                .avisar("Não deu para colar aqui. O texto está no histórico", 3.5),
        }
    }

    pub fn cancelar(&self) {
        let gravacao = {
            let mut i = self.interno.lock().unwrap();
            if i.fase != Fase::Gravando {
                return;
            }
            // Ninguém começa outro ditado enquanto a fonte deste não saiu (I-1).
            i.fase = Fase::Processando;
            i.gravacao.take()
        };
        if let Some(g) = gravacao {
            let Gravacao {
                ditado: d,
                mut captura,
                destino,
                bomba,
                ..
            } = g;
            captura.parar();
            drop(captura);
            if let Some(dest) = destino {
                dest.lock().unwrap().take();
            }
            let integracao = self.integracao.clone();
            let pasta = self.sistema.pasta_de_audio();
            thread::spawn(move || {
                if let Ok((s, _)) = bomba.join() {
                    s.cancelar();
                }
                let _ = integracao.descartar_ditado(d.id);
                apagar_audio(&pasta, &d.audio);
            });
        }
        {
            let mut i = self.interno.lock().unwrap();
            i.fase = Fase::Parado;
            i.maos_livres = false;
        }
        self.pilula.mostrar(FasePilula::Ociosa);
        self.mudou("estado");
    }

    pub fn colar_ultimo(&self) {
        if self.fase() != Fase::Parado {
            return;
        }
        match self.integracao.ultimo_texto() {
            Ok(Some(t)) if !t.is_empty() => self.avisar_sem_colar(self.sistema.colar(&t)),
            _ => self.pilula.avisar("Nenhum ditado para colar", 1.6),
        }
    }

    /// Passa de novo pelo reconhecedor o áudio guardado de um ditado que falhou ou ficou parcial.
    pub fn retranscrever(self: &Arc<Self>, id: i64) -> Result<(), String> {
        {
            let mut i = self.interno.lock().unwrap();
            if i.fase != Fase::Parado {
                return Err("Termine o ditado atual antes".into());
            }
            i.fase = Fase::Processando;
        }
        self.pilula.mostrar(FasePilula::Processando);
        self.mudou("estado");
        let c = self.clone();
        thread::spawn(move || {
            let aviso = c
                .integracao
                .retranscrever(id, &c.sistema.pasta_de_audio(), c.prefs.limpeza_ia())
                .unwrap_or_else(|e| {
                    nota(format!("retranscrever {id}: {e}"));
                    "Ainda não consegui transcrever"
                });
            c.interno.lock().unwrap().fase = Fase::Parado;
            c.pilula.avisar(aviso, 3.0);
            c.mudou("estado");
            c.mudou("historico");
        });
        Ok(())
    }

    /// Saída do app. Fecha o microfone, espera a gravação escrever no WAV e na sessão tudo o que
    /// já chegou (inclusive os últimos 100 ms) e, se um ditado estiver sendo processado, espera ele
    /// terminar, tudo dentro de `prazo`. Um ditado não concluído fica "gravando" no banco com a
    /// reserva de trechos e o WAV, e volta ao histórico pela recuperação na próxima abertura.
    pub fn desligar(&self, prazo: Duration) {
        let limite = Instant::now() + prazo;
        let g = self.interno.lock().unwrap().gravacao.take();
        if let Some(g) = g {
            let Gravacao {
                ditado,
                mut captura,
                destino,
                bomba,
                ..
            } = g;
            captura.parar();
            drop(captura);
            if let Some(dest) = destino {
                dest.lock().unwrap().take();
            }
            let (tx, rx) = mpsc::channel();
            thread::spawn(move || {
                let _ = tx.send(bomba.join().map(|(_, n)| n));
            });
            match rx.recv_timeout(limite.saturating_duration_since(Instant::now())) {
                Ok(Ok(n)) => nota(format!(
                    "ditado {}: saída com {:.1} s gravados; volta pela recuperação",
                    ditado.id,
                    n as f64 / crate::audio::TAXA as f64
                )),
                _ => nota(format!(
                    "ditado {}: a gravação não terminou no prazo de saída",
                    ditado.id
                )),
            }
            let mut i = self.interno.lock().unwrap();
            i.fase = Fase::Parado;
            i.maos_livres = false;
        }
        while self.fase() == Fase::Processando && Instant::now() < limite {
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn limpar_audios_antigos(&self) {
        let dias = self.prefs.guardar_audio_dias() as u64;
        let limite = Duration::from_secs(dias.max(1) * 86_400);
        let Ok(lista) = std::fs::read_dir(self.sistema.pasta_de_audio()) else {
            return;
        };
        for e in lista.flatten() {
            let velho = e
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .map(|i| i > limite)
                .unwrap_or(false);
            if velho && e.path().extension().map(|x| x == "wav").unwrap_or(false) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

fn apagar_audio(pasta: &std::path::Path, nome: &str) {
    if nome.is_empty() || nome.contains(['/', '\\']) {
        return;
    }
    let _ = std::fs::remove_file(pasta.join(nome));
}

/// Liga a máquina do atalho ao controlador sem ciclo de referência.
struct Ponte(Weak<Controlador>);

impl Acoes for Ponte {
    fn comecar(&mut self) -> bool {
        self.0.upgrade().map(|c| c.comecar(false)).unwrap_or(false)
    }
    fn terminar(&mut self) {
        if let Some(c) = self.0.upgrade() {
            c.terminar(true)
        }
    }
    fn cancelar(&mut self) {
        if let Some(c) = self.0.upgrade() {
            c.cancelar()
        }
    }
    fn maos_livres(&mut self) {
        if let Some(c) = self.0.upgrade() {
            c.entrar_em_maos_livres()
        }
    }
    fn colar_ultimo(&mut self) {
        if let Some(c) = self.0.upgrade() {
            c.colar_ultimo()
        }
    }
    fn interromper(&mut self) {
        if let Some(c) = self.0.upgrade() {
            c.terminar(false)
        }
    }
}

#[cfg(test)]
mod testes {
    use super::*;
    use crate::integracao::dubles::{voz, MotoresDuble};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    #[derive(Default)]
    struct PilulaDuble {
        avisos: Mutex<Vec<String>>,
        fases: Mutex<Vec<FasePilula>>,
    }
    impl Avisos for PilulaDuble {
        fn mostrar(&self, fase: FasePilula) {
            self.fases.lock().unwrap().push(fase);
        }
        fn avisar(&self, texto: &str, _s: f64) {
            self.avisos.lock().unwrap().push(texto.into());
        }
        fn nivel(&self, _n: f32) {}
        fn segundos(&self, _s: u64) {}
        fn atalho(&self, _s: &str) {}
        fn sempre_visivel(&self, _s: bool) {}
    }

    /// Microfone de mentira: entrega `pcm` em blocos de 100 ms numa thread até ser parado.
    struct FonteDuble {
        parar: Arc<AtomicBool>,
        fio: Option<thread::JoinHandle<()>>,
    }
    impl FonteAberta for FonteDuble {
        fn parar(&mut self) {
            self.parar.store(true, Ordering::Release);
            if let Some(f) = self.fio.take() {
                let _ = f.join();
            }
        }
    }
    fn tocar(pcm: Vec<f32>, ao_bloco: AoBlocoDeAudio) -> FonteDuble {
        tocar_com(pcm, false, ao_bloco)
    }
    /// `tempo_real`: cada bloco de 100 ms chega 100 ms depois do anterior, como num microfone.
    fn tocar_com(pcm: Vec<f32>, tempo_real: bool, mut ao_bloco: AoBlocoDeAudio) -> FonteDuble {
        let parar = Arc::new(AtomicBool::new(false));
        let p = parar.clone();
        let fio = thread::spawn(move || {
            for b in pcm.chunks(1600) {
                if tempo_real {
                    thread::sleep(Duration::from_millis(100));
                }
                if p.load(Ordering::Acquire) {
                    break;
                }
                ao_bloco(b.to_vec(), 0.3);
            }
        });
        FonteDuble {
            parar,
            fio: Some(fio),
        }
    }

    struct SistemaDuble {
        pcm: Vec<f32>,
        microfone: bool,
        colagem: Insercao,
        colados: Mutex<Vec<String>>,
        aberturas: AtomicUsize,
        pasta: PathBuf,
        microfone_falha: bool,
        /// Microfone em tempo real ouvindo o "mundo" de `fonte_do_mundo`, com atraso de abertura.
        mundo: Option<(Instant, Duration)>,
    }
    impl Sistema for SistemaDuble {
        fn microfone_disponivel(&self) -> bool {
            self.microfone
        }
        fn abrir_microfone(
            &self,
            _n: &str,
            ao_bloco: AoBlocoDeAudio,
        ) -> Result<Box<dyn FonteAberta>, String> {
            self.aberturas.fetch_add(1, Ordering::Relaxed);
            if self.microfone_falha {
                return Err("dispositivo ocupado".into());
            }
            if let Some((t0, atraso)) = self.mundo {
                // Abertura lenta, como o WASAPI ou um headset trocando de perfil.
                thread::sleep(atraso);
                return Ok(Box::new(fonte_do_mundo(t0, ao_bloco)));
            }
            Ok(Box::new(tocar(self.pcm.clone(), ao_bloco)))
        }
        fn app_em_foco(&self) -> (String, String) {
            ("notepad.exe".into(), "notepad".into())
        }
        fn colar(&self, texto: &str) -> Insercao {
            self.colados.lock().unwrap().push(texto.into());
            self.colagem
        }
        fn pasta_de_audio(&self) -> PathBuf {
            self.pasta.clone()
        }
    }

    struct Cena {
        _tmp: tempfile::TempDir,
        c: Arc<Controlador>,
        pilula: Arc<PilulaDuble>,
        sistema: Arc<SistemaDuble>,
    }

    fn cena(pcm: Vec<f32>, ajustar: impl FnOnce(&mut SistemaDuble)) -> Cena {
        let tmp = crate::caminhos::pasta_de_teste();
        let prefs = Arc::new(Preferencias::abrir(tmp.path().join("p.json")));
        let integracao = Arc::new(
            Integracao::abrir(&tmp.path().join("d.sqlite"), MotoresDuble::novo()).unwrap(),
        );
        let mut s = SistemaDuble {
            pcm,
            microfone: true,
            colagem: Insercao::Colado,
            colados: Mutex::new(Vec::new()),
            aberturas: AtomicUsize::new(0),
            pasta: tmp.path().join("Audio"),
            microfone_falha: false,
            mundo: None,
        };
        std::fs::create_dir_all(&s.pasta).unwrap();
        ajustar(&mut s);
        let sistema = Arc::new(s);
        let pilula = Arc::new(PilulaDuble::default());
        let c = Controlador::new(prefs, integracao, pilula.clone(), sistema.clone(), |_| {});
        Cena {
            _tmp: tmp,
            c,
            pilula,
            sistema,
        }
    }

    fn esperar_parado(c: &Controlador) {
        let inicio = Instant::now();
        while c.fase() != Fase::Parado {
            assert!(
                inicio.elapsed() < Duration::from_secs(10),
                "o ditado não terminou"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn ditar(cena: &Cena, colar: bool) {
        assert!(cena.c.comecar(false));
        thread::sleep(Duration::from_millis(400));
        cena.c.terminar(colar);
        esperar_parado(&cena.c);
    }

    #[test]
    fn ditado_completo_cola_grava_historico_e_guarda_audio() {
        let cena = cena(voz(2.0, 0.5), |_| {});
        ditar(&cena, true);
        let colados = cena.sistema.colados.lock().unwrap().clone();
        assert_eq!(colados.len(), 1);
        assert!(
            colados[0].contains("palavra") && colados[0].ends_with(' '),
            "{colados:?}"
        );
        let h = cena.c.integracao.historico(10, "").unwrap();
        assert_eq!(h[0]["estado"], "concluido");
        assert_eq!(h[0]["appNome"], "notepad");
        assert!(cena
            .sistema
            .pasta
            .join(h[0]["audio"].as_str().unwrap())
            .is_file());
        assert_eq!(
            cena.pilula.fases.lock().unwrap().last(),
            Some(&FasePilula::Ociosa)
        );
    }

    #[test]
    fn cancelar_nao_guarda_nada() {
        let cena = cena(voz(2.0, 0.5), |_| {});
        assert!(cena.c.comecar(false));
        thread::sleep(Duration::from_millis(200));
        cena.c.cancelar();
        thread::sleep(Duration::from_millis(300));
        assert!(cena.sistema.colados.lock().unwrap().is_empty());
        assert!(cena
            .c
            .integracao
            .historico(10, "")
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty());
        assert_eq!(
            std::fs::read_dir(&cena.sistema.pasta).unwrap().count(),
            0,
            "WAV do ditado cancelado ficou"
        );
    }

    #[test]
    fn silencio_avisa_e_nao_guarda() {
        let cena = cena(vec![0.0; 16_000], |_| {});
        ditar(&cena, true);
        assert!(cena
            .pilula
            .avisos
            .lock()
            .unwrap()
            .iter()
            .any(|a| a == "Não ouvi nada"));
        assert!(cena
            .c
            .integracao
            .historico(10, "")
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn sem_microfone_nao_comeca() {
        let cena = cena(Vec::new(), |s| s.microfone = false);
        assert!(!cena.c.comecar(false));
        assert!(cena.pilula.avisos.lock().unwrap()[0].starts_with("Microfone indisponível"));
        assert_eq!(cena.c.fase(), Fase::Parado);
    }

    #[test]
    fn colagem_recusada_avisa_que_esta_no_historico() {
        let cena = cena(voz(2.0, 0.5), |s| s.colagem = Insercao::Recusado);
        ditar(&cena, true);
        assert!(cena
            .pilula
            .avisos
            .lock()
            .unwrap()
            .iter()
            .any(|a| a.contains("está no histórico")));
        assert_eq!(
            cena.c.integracao.historico(10, "").unwrap()[0]["estado"],
            "concluido"
        );
    }

    #[test]
    fn interromper_guarda_sem_colar_e_colar_ultimo_usa_o_historico() {
        let cena = cena(voz(2.0, 0.5), |_| {});
        ditar(&cena, false);
        assert!(cena.sistema.colados.lock().unwrap().is_empty());
        cena.c.colar_ultimo();
        assert_eq!(cena.sistema.colados.lock().unwrap().len(), 1);
    }

    struct ReuniaoDuble {
        pcm: Vec<f32>,
        derivacoes: AtomicUsize,
        ao_acabar: Mutex<Option<AoAcabar>>,
        parar: Mutex<Option<Arc<AtomicBool>>>,
        tempo_real: bool,
    }
    impl ReuniaoDuble {
        fn novo(pcm: Vec<f32>) -> Arc<Self> {
            Arc::new(Self {
                pcm,
                derivacoes: AtomicUsize::new(0),
                ao_acabar: Mutex::new(None),
                parar: Mutex::new(None),
                tempo_real: false,
            })
        }
        /// Microfone da reunião em tempo real: o que a emenda corta pelo relógio precisa disso.
        fn em_tempo_real(pcm: Vec<f32>) -> Arc<Self> {
            Arc::new(Self {
                tempo_real: true,
                ..Arc::into_inner(Self::novo(pcm)).unwrap()
            })
        }
        /// Como `Lancamento::soltar_capturas`: avisa com a derivação viva, fecha a derivação e
        /// roda a segunda fase.
        fn acabar(&self) {
            let f = self
                .ao_acabar
                .lock()
                .unwrap()
                .take()
                .expect("ao_acabar registrado");
            let depois = f();
            if let Some(p) = self.parar.lock().unwrap().take() {
                p.store(true, Ordering::Release);
            }
            if let Some(d) = depois {
                d();
            }
        }
    }
    impl MicrofoneCompartilhado for ReuniaoDuble {
        fn derivar(
            &self,
            ao_bloco: AoBlocoDeAudio,
            ao_acabar: AoAcabar,
        ) -> Option<Box<dyn FonteAberta>> {
            *self.ao_acabar.lock().unwrap() = Some(ao_acabar);
            self.derivacoes.fetch_add(1, Ordering::Relaxed);
            let fonte = tocar_com(self.pcm.clone(), self.tempo_real, ao_bloco);
            *self.parar.lock().unwrap() = Some(fonte.parar.clone());
            Some(Box::new(fonte))
        }
    }

    /// I5: com reunião gravando o microfone, o ditado deriva dela e não abre outra captura.
    #[test]
    fn ditado_durante_reuniao_usa_o_microfone_da_reuniao() {
        let cena = cena(Vec::new(), |_| {});
        let reuniao = ReuniaoDuble::novo(voz(2.0, 0.5));
        cena.c.compartilhar_microfone(reuniao.clone());
        ditar(&cena, true);
        assert_eq!(reuniao.derivacoes.load(Ordering::Relaxed), 1);
        assert_eq!(
            cena.sistema.aberturas.load(Ordering::Relaxed),
            0,
            "abriu uma segunda captura"
        );
        assert!(cena.sistema.colados.lock().unwrap()[0].contains("palavra"));
    }

    /// I-2: a reunião acaba no meio do ditado. O ditado troca para o microfone próprio e o que se
    /// fala depois entra no texto.
    #[test]
    fn reuniao_acaba_no_meio_e_o_ditado_segue_no_microfone_proprio() {
        let cena = cena(voz(1.5, 0.5), |_| {});
        let reuniao = ReuniaoDuble::em_tempo_real(voz(3.0, 0.0));
        cena.c.compartilhar_microfone(reuniao.clone());
        assert!(cena.c.comecar(true));
        thread::sleep(Duration::from_millis(1100));
        reuniao.acabar();
        assert_eq!(
            cena.sistema.aberturas.load(Ordering::Relaxed),
            1,
            "não abriu o microfone próprio"
        );
        assert_eq!(cena.c.fase(), Fase::Gravando, "o ditado deveria continuar");
        thread::sleep(Duration::from_millis(300));
        cena.c.terminar(true);
        esperar_parado(&cena.c);
        let colado = cena.sistema.colados.lock().unwrap()[0].clone();
        assert!(
            // Só a derivação daria 2 palavras e só o microfone próprio, no máximo 3.
            colado.split_whitespace().count() >= 4,
            "faltou fala de uma das fontes: {colado}"
        );
    }

    /// I-2: se o microfone próprio não abrir, o ditado termina com o que já tinha e a pílula avisa.
    #[test]
    fn reuniao_acaba_e_microfone_nao_abre_encerra_com_o_que_tinha() {
        let cena = cena(Vec::new(), |s| s.microfone_falha = true);
        let reuniao = ReuniaoDuble::em_tempo_real(voz(1.5, 0.5));
        cena.c.compartilhar_microfone(reuniao.clone());
        assert!(cena.c.comecar(true));
        thread::sleep(Duration::from_millis(700));
        reuniao.acabar();
        esperar_parado(&cena.c);
        assert!(cena
            .pilula
            .avisos
            .lock()
            .unwrap()
            .iter()
            .any(|a| a.starts_with("A reunião acabou")));
        let colado = cena.sistema.colados.lock().unwrap()[0].clone();
        assert!(
            colado.to_lowercase().contains("palavra"),
            "perdeu o que já tinha: {colado:?}"
        );
    }

    struct SemReuniao;
    impl MicrofoneCompartilhado for SemReuniao {
        fn derivar(&self, _a: AoBlocoDeAudio, _f: AoAcabar) -> Option<Box<dyn FonteAberta>> {
            None
        }
    }

    /// A fala do "mundo": a amostra global `k` (desde `t0`) vale `k mod 32000`, gravada no WAV de
    /// 16 bits sem perda. Lacuna e repetição aparecem como saltos na sequência.
    fn amostra_do_mundo(k: u64) -> f32 {
        ((k % 32_000) as f32 + 0.5) / 32_767.0
    }
    fn indice(t0: Instant) -> u64 {
        (t0.elapsed().as_secs_f64() * audio::TAXA as f64) as u64
    }

    /// Fonte em tempo real: a cada ~10 ms entrega as amostras do mundo desde a última entrega.
    fn fonte_do_mundo(t0: Instant, mut ao_bloco: AoBlocoDeAudio) -> FonteDuble {
        let parar = Arc::new(AtomicBool::new(false));
        let p = parar.clone();
        let fio = thread::spawn(move || {
            let mut k = indice(t0);
            while !p.load(Ordering::Acquire) {
                thread::sleep(Duration::from_millis(10));
                let fim = indice(t0);
                ao_bloco((k..fim).map(amostra_do_mundo).collect(), 0.5);
                k = fim;
            }
        });
        FonteDuble {
            parar,
            fio: Some(fio),
        }
    }

    /// Capturador da reunião em tempo real: o microfone ouve o mesmo mundo; o áudio do sistema
    /// fica mudo. Soltar o guarda para a fonte.
    struct CapturaDoMundo(Instant);
    impl crate::lancamento::Capturador for CapturaDoMundo {
        fn abrir(
            &self,
            fonte: Fonte,
            relogio: Instant,
            mut ao_bloco: crate::lancamento::AoBloco,
        ) -> Result<Box<dyn Send>, String> {
            struct Guarda(FonteDuble);
            impl Drop for Guarda {
                fn drop(&mut self) {
                    self.0.parar();
                }
            }
            let t0 = if fonte == Fonte::Microfone {
                self.0
            } else {
                // Sem fonte do mundo para o sistema: um relógio no futuro não entrega nada.
                Instant::now() + Duration::from_secs(3600)
            };
            Ok(Box::new(Guarda(fonte_do_mundo(
                t0,
                Box::new(move |b, n| {
                    if b.is_empty() {
                        return;
                    }
                    let fim = relogio.elapsed().as_secs_f64();
                    ao_bloco(fim - b.len() as f64 / audio::TAXA as f64, b, n)
                }),
            ))))
        }
    }

    /// Saltos na sequência do mundo gravada no WAV: (amostras faltando, amostras repetidas).
    fn lacunas_e_repeticoes(wav: &std::path::Path) -> (u64, u64, usize) {
        let bytes = std::fs::read(wav).unwrap();
        let k: Vec<u64> = bytes[44..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| i16::from_le_bytes(*b) as u64)
            .collect();
        let (mut falta, mut repete) = (0, 0);
        for par in k.windows(2) {
            let d = (par[1] + 32_000 - par[0]) % 32_000;
            if d == 0 || d > 16_000 {
                repete += (32_000 - d) % 32_000 + 1;
            } else {
                falta += d - 1;
            }
        }
        (falta, repete, k.len())
    }

    /// N-1, de ponta a ponta com o serviço de reuniões real: a reunião acaba no meio de um ditado
    /// derivado e o microfone próprio leva 150 ms para abrir. A fala do mundo chega inteira ao
    /// ditado, sem lacuna nem repetição na emenda (tolerância de 10 ms para o agendamento das
    /// threads). Antes, a reunião fechava a captura antes de avisar e os 150 ms se perdiam.
    #[test]
    fn reuniao_acaba_no_meio_e_a_emenda_nao_perde_nem_duplica() {
        let t0 = Instant::now();
        let cena = cena(Vec::new(), |s| {
            s.mundo = Some((t0, Duration::from_millis(150)))
        });
        let tmp = crate::caminhos::pasta_de_teste();
        let reuniao = crate::lancamento::Lancamento::new(
            &tmp.path().join("r.sqlite"),
            &tmp.path().join("ReunioesAudio"),
            MotoresDuble::novo(),
            Arc::new(CapturaDoMundo(t0)),
            Arc::new(|t: &str| t.to_string()),
            Arc::new(|_, _| {}),
        )
        .unwrap();
        cena.c.compartilhar_microfone(reuniao.clone());
        let id = reuniao.iniciar_reuniao("macEMicrofone").unwrap();
        thread::sleep(Duration::from_millis(100));
        assert!(cena.c.comecar(true));
        thread::sleep(Duration::from_millis(400));
        reuniao.encerrar(&id).unwrap();
        assert_eq!(cena.sistema.aberturas.load(Ordering::Relaxed), 1);
        assert_eq!(cena.c.fase(), Fase::Gravando, "o ditado deveria continuar");
        thread::sleep(Duration::from_millis(400));
        cena.c.terminar(false);
        esperar_parado(&cena.c);
        reuniao.desligar(Duration::from_secs(2));
        let h = cena.c.integracao.historico(10, "").unwrap();
        let wav = cena.sistema.pasta.join(h[0]["audio"].as_str().unwrap());
        let (falta, repete, total) = lacunas_e_repeticoes(&wav);
        assert!(total > 12_000, "ditado curto demais: {total} amostras");
        assert!(
            falta <= 160 && repete <= 160,
            "emenda com {falta} amostras faltando e {repete} repetidas"
        );
    }

    /// N-1: com o microfone recusando a segunda abertura enquanto a reunião o segura, a troca
    /// acontece depois que a reunião solta, e a pílula avisa que um trecho pode ter ficado de fora.
    #[test]
    fn microfone_ocupado_pela_reuniao_abre_depois_e_avisa() {
        struct OcupadoUmaVez {
            dentro: SistemaDuble,
            tentativas: AtomicUsize,
        }
        impl Sistema for OcupadoUmaVez {
            fn microfone_disponivel(&self) -> bool {
                true
            }
            fn abrir_microfone(
                &self,
                n: &str,
                ao_bloco: AoBlocoDeAudio,
            ) -> Result<Box<dyn FonteAberta>, String> {
                if self.tentativas.fetch_add(1, Ordering::Relaxed) == 0 {
                    return Err("dispositivo ocupado".into());
                }
                self.dentro.abrir_microfone(n, ao_bloco)
            }
            fn app_em_foco(&self) -> (String, String) {
                self.dentro.app_em_foco()
            }
            fn colar(&self, texto: &str) -> Insercao {
                self.dentro.colar(texto)
            }
            fn pasta_de_audio(&self) -> PathBuf {
                self.dentro.pasta_de_audio()
            }
        }
        let tmp = crate::caminhos::pasta_de_teste();
        let prefs = Arc::new(Preferencias::abrir(tmp.path().join("p.json")));
        let integracao = Arc::new(
            Integracao::abrir(&tmp.path().join("d.sqlite"), MotoresDuble::novo()).unwrap(),
        );
        let pasta = tmp.path().join("Audio");
        std::fs::create_dir_all(&pasta).unwrap();
        let sistema = Arc::new(OcupadoUmaVez {
            dentro: SistemaDuble {
                pcm: voz(1.5, 0.5),
                microfone: true,
                colagem: Insercao::Colado,
                colados: Mutex::new(Vec::new()),
                aberturas: AtomicUsize::new(0),
                pasta,
                microfone_falha: false,
                mundo: None,
            },
            tentativas: AtomicUsize::new(0),
        });
        let pilula = Arc::new(PilulaDuble::default());
        let c = Controlador::new(prefs, integracao, pilula.clone(), sistema.clone(), |_| {});
        let reuniao = ReuniaoDuble::em_tempo_real(voz(3.0, 0.0));
        c.compartilhar_microfone(reuniao.clone());
        assert!(c.comecar(true));
        thread::sleep(Duration::from_millis(200));
        reuniao.acabar();
        assert_eq!(sistema.tentativas.load(Ordering::Relaxed), 2);
        assert_eq!(c.fase(), Fase::Gravando, "o ditado deveria continuar");
        assert!(pilula
            .avisos
            .lock()
            .unwrap()
            .iter()
            .any(|a| a.starts_with("Troquei de microfone")));
        thread::sleep(Duration::from_millis(300));
        c.terminar(true);
        esperar_parado(&c);
        assert!(sistema.dentro.colados.lock().unwrap()[0].contains("palavra"));
    }

    #[test]
    fn sem_reuniao_abre_o_proprio_microfone() {
        let cena = cena(voz(2.0, 0.5), |_| {});
        cena.c.compartilhar_microfone(Arc::new(SemReuniao));
        ditar(&cena, true);
        assert_eq!(cena.sistema.aberturas.load(Ordering::Relaxed), 1);
        assert_eq!(cena.sistema.colados.lock().unwrap().len(), 1);
    }

    /// Saída no meio do ditado: a gravação drena inteira no WAV e o ditado volta pela recuperação.
    #[test]
    fn desligar_durante_ditado_deixa_para_a_recuperacao() {
        let cena = cena(voz(3.0, 0.5), |_| {});
        assert!(cena.c.comecar(false));
        thread::sleep(Duration::from_millis(400));
        cena.c.desligar(Duration::from_secs(5));
        assert_eq!(cena.c.fase(), Fase::Parado);
        // O trecho em andamento ainda não virou reserva de texto: o ditado volta como "falhou",
        // com o WAV inteiro guardado para retranscrever (mesma regra do Mac).
        cena.c
            .integracao
            .recuperar_interrompidos(&cena.sistema.pasta)
            .unwrap();
        let h = cena.c.integracao.historico(10, "").unwrap();
        assert_eq!(h[0]["estado"], "falhou", "{h}");
        let wav = cena.sistema.pasta.join(h[0]["audio"].as_str().unwrap());
        assert!(
            std::fs::metadata(&wav).unwrap().len() > 44 + 2 * 16_000 * 3,
            "WAV incompleto"
        );
    }
}
