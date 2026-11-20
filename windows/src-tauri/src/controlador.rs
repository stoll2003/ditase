//! Ciclo do ditado (porta de `Controlador.swift`): atalho, microfone, sessão, banco, colagem e
//! pílula. A sessão (Parakeet, regras, dicionário e limpeza opcional) e o banco vêm de
//! [`crate::integracao`]. Sem modelo de fala pronto, o ditado não começa e a pílula diz por quê.
use std::{
    sync::{mpsc, Arc, Mutex, OnceLock, Weak},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use crate::{
    atalho::{Acoes, Atalho, Entrada},
    audio::{self, Captura, EscritorWav, Fonte},
    caminhos::{self, nota},
    integracao::{Ditado, Integracao, SessaoDeDitado},
    nativo::{self, Insercao},
    pilula::{Fase as FasePilula, Pilula},
    preferencias::Preferencias,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fase { Parado, Gravando, Processando }

impl Fase { pub fn id(self) -> &'static str { match self { Fase::Parado => "parado", Fase::Gravando => "gravando", Fase::Processando => "processando" } } }

struct Gravacao {
    ditado: Ditado,
    inicio: Instant,
    captura: Captura,
    bomba: thread::JoinHandle<(Box<dyn SessaoDeDitado>, usize)>,
    silencio: Arc<Mutex<f64>>,
}

struct Interno { fase: Fase, maos_livres: bool, gravacao: Option<Gravacao> }

pub struct Controlador {
    pub prefs: Arc<Preferencias>,
    pub integracao: Arc<Integracao>,
    pub pilula: Arc<Pilula>,
    pub atalho: OnceLock<Atalho>,
    pub atalho_instalado: Mutex<bool>,
    interno: Mutex<Interno>,
    /// Avisa a interface: "estado", "historico" ou "dicionario".
    ao_mudar: Box<dyn Fn(&str) + Send + Sync>,
}

impl Controlador {
    pub fn new(prefs: Arc<Preferencias>, integracao: Arc<Integracao>, pilula: Arc<Pilula>, ao_mudar: impl Fn(&str) + Send + Sync + 'static) -> Arc<Self> {
        Arc::new(Self {
            prefs, integracao, pilula, atalho: OnceLock::new(), atalho_instalado: Mutex::new(false),
            interno: Mutex::new(Interno { fase: Fase::Parado, maos_livres: false, gravacao: None }),
            ao_mudar: Box::new(ao_mudar),
        })
    }

    /// Liga atalho, manutenção e relógio. Chamado uma vez na abertura.
    pub fn ligar(self: &Arc<Self>) {
        match self.integracao.recuperar_interrompidos(&caminhos::audios()) {
            Ok(n) if n > 0 => nota(format!("recuperação: {n} ditado(s) interrompido(s) voltaram ao histórico")),
            Err(e) => nota(format!("recuperação: {e}")),
            _ => {}
        }
        self.limpar_audios_antigos();
        let atalho = Atalho::iniciar(self.prefs.atalho().vk(), Ponte(Arc::downgrade(self)));
        let instalado = nativo::gancho::instalar(atalho.visao.clone(), atalho.emissor(), atalho.relogio);
        *self.atalho_instalado.lock().unwrap() = instalado;
        let _ = self.atalho.set(atalho);
        let fraco = Arc::downgrade(self);
        thread::Builder::new().name("ditase.relogio".into()).spawn(move || loop {
            thread::sleep(Duration::from_millis(500));
            let Some(c) = fraco.upgrade() else { break };
            c.acompanhar();
        }).expect("relógio");
    }

    pub fn fase(&self) -> Fase { self.interno.lock().unwrap().fase }
    pub fn maos_livres(&self) -> bool { self.interno.lock().unwrap().maos_livres }
    fn mudou(&self, assunto: &str) { (self.ao_mudar)(assunto) }
    fn avisar_atalho(&self, e: Entrada) { if let Some(a) = self.atalho.get() { a.enviar(e); } }

    pub fn preferencia_mudou(&self, chave: &str) {
        match chave {
            "atalho" => {
                let t = self.prefs.atalho();
                if let Some(a) = self.atalho.get() { a.trocar_tecla(t.vk()); }
                self.pilula.atalho(t.simbolo());
            }
            "pilulaSempreVisivel" => self.pilula.sempre_visivel(self.prefs.pilula_sempre_visivel()),
            _ => {}
        }
        self.mudou("estado");
    }

    // MARK: Ciclo do ditado

    pub fn comecar(self: &Arc<Self>, maos_livres: bool) -> bool {
        let mut i = self.interno.lock().unwrap();
        if i.fase != Fase::Parado { return false; }
        if !audio::microfone_disponivel() {
            drop(i);
            self.pilula.avisar("Microfone indisponível. Confira Privacidade > Microfone", 3.5);
            return false;
        }
        let (app, app_nome) = nativo::sistema::app_em_foco();
        let mut d = Ditado {
            iniciado_em: SystemTime::now().duration_since(UNIX_EPOCH).map(|x| x.as_secs_f64()).unwrap_or(0.0),
            app, app_nome, estado: "gravando".into(), motor: "Parakeet".into(), ..Default::default()
        };
        d.id = match self.integracao.inserir_ditado(&d) {
            Ok(id) => id,
            Err(e) => { nota(format!("banco: {e}")); drop(i); self.pilula.avisar("Erro ao abrir o histórico", 3.0); return false; }
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
        if limpeza { self.integracao.motores.aquecer(); }
        let caminho = (self.prefs.guardar_audio_dias() > 0).then(|| caminhos::audios().join(format!("{}.wav", d.id)));
        let mut escritor = caminho.as_ref().and_then(|c| EscritorWav::criar(c).map_err(|e| nota(format!("wav: {e}"))).ok());
        if escritor.is_some() {
            d.audio = caminho.as_ref().and_then(|c| c.file_name()).map(|n| n.to_string_lossy().into()).unwrap_or_default();
            if let Err(e) = self.integracao.definir_audio(d.id, &d.audio) { nota(format!("ditado {}: áudio não registrado ({e})", d.id)); }
        }

        let (tx, rx) = mpsc::channel::<Vec<f32>>();
        let pilula = self.pilula.clone();
        let captura = match Captura::iniciar(Fonte::Microfone, &self.prefs.microfone(), move |bloco, nivel| {
            pilula.empurrar_nivel(nivel);
            let _ = tx.send(bloco);
        }) {
            Ok(c) => c,
            Err(e) => {
                nota(format!("microfone: {e}"));
                let _ = self.integracao.descartar_ditado(d.id);
                // O WAV de 44 bytes criado acima não fica órfão em Audio/.
                drop(escritor);
                if let Some(c) = &caminho { let _ = std::fs::remove_file(c); }
                drop(i);
                self.pilula.avisar("Microfone indisponível", 3.0);
                return false;
            }
        };
        let silencio = Arc::new(Mutex::new(0.0f64));
        let silencio_bomba = silencio.clone();
        let bomba = thread::Builder::new().name("ditase.bomba".into()).spawn(move || {
            let mut sessao = sessao;
            let mut total = 0usize;
            for bloco in rx {
                total += bloco.len();
                if let Some(w) = escritor.as_mut() { if let Err(e) = w.gravar(&bloco) { nota(format!("wav: {e}")); escritor = None; } }
                sessao.alimentar(&bloco);
                *silencio_bomba.lock().unwrap() = sessao.silencio();
            }
            (sessao, total)
        }).expect("bomba");

        let id = d.id;
        i.gravacao = Some(Gravacao { ditado: d, inicio: Instant::now(), captura, bomba, silencio });
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
                let fase = if i.maos_livres { FasePilula::MaosLivres } else { FasePilula::Gravando };
                drop(i);
                c.pilula.mostrar(fase);
            }
        });
        self.mudou("estado");
        true
    }

    pub fn entrar_em_maos_livres(&self) {
        let mut i = self.interno.lock().unwrap();
        if i.fase != Fase::Gravando { return; }
        i.maos_livres = true;
        drop(i);
        self.pilula.mostrar(FasePilula::MaosLivres);
        self.mudou("estado");
    }

    /// Chamado pela interface e pela pílula: começa em mãos livres ou termina o que estiver gravando.
    pub fn alternar(self: &Arc<Self>) {
        match self.fase() {
            Fase::Parado => { if self.comecar(true) { self.avisar_atalho(Entrada::IniciadoEmMaosLivres); } }
            Fase::Gravando => { self.avisar_atalho(Entrada::EncerradoPorFora); self.terminar(true); }
            Fase::Processando => {}
        }
    }

    fn acompanhar(self: &Arc<Self>) {
        let (decorrido, maos_livres, silencio) = {
            let i = self.interno.lock().unwrap();
            if i.fase != Fase::Gravando { return; }
            let Some(g) = i.gravacao.as_ref() else { return };
            let valores = (g.inicio.elapsed().as_secs_f64(), i.maos_livres, *g.silencio.lock().unwrap());
            valores
        };
        self.pilula.segundos(decorrido as u64);
        if decorrido >= self.prefs.limite_minutos() * 60.0 {
            nota(format!("ditado encerrado pelo limite de {} min", self.prefs.limite_minutos()));
            self.avisar_atalho(Entrada::EncerradoPorFora);
            self.terminar(true);
            return;
        }
        let limite = self.prefs.parar_em_silencio_minutos() * 60.0;
        if maos_livres && limite > 0.0 && silencio >= limite {
            nota(format!("ditado encerrado por {} s de silêncio", silencio as u64));
            self.avisar_atalho(Entrada::EncerradoPorFora);
            self.terminar(true);
        }
    }

    pub fn terminar(self: &Arc<Self>, colar: bool) {
        let gravacao = {
            let mut i = self.interno.lock().unwrap();
            if i.fase != Fase::Gravando { return; }
            i.fase = Fase::Processando;
            i.gravacao.take()
        };
        let Some(mut g) = gravacao else { return };
        let soltou = Instant::now();
        g.captura.parar();
        self.pilula.mostrar(FasePilula::Processando);
        self.mudou("estado");
        let c = self.clone();
        thread::spawn(move || {
            let mut d = g.ditado;
            d.duracao = soltou.duration_since(g.inicio).as_secs_f64();
            match g.bomba.join() {
                Ok((sessao, _)) => {
                    let r = sessao.finalizar();
                    d.latencia = soltou.elapsed().as_secs_f64();
                    c.concluir(d, r, colar);
                }
                Err(_) => {
                    nota(format!("ditado {}: a gravação parou com erro", d.id));
                    let _ = c.integracao.marcar_falha(&d);
                    c.pilula.avisar("Não consegui transcrever", 3.0);
                }
            }
            { let mut i = c.interno.lock().unwrap(); i.fase = Fase::Parado; i.maos_livres = false; }
            c.mudou("estado");
            c.mudou("historico");
        });
    }

    fn concluir(&self, mut d: Ditado, r: crate::integracao::ResultadoDitado, colar: bool) {
        if r.texto.is_empty() {
            if r.falhas == 0 {
                // Silêncio de verdade: não há o que guardar.
                let _ = self.integracao.descartar_ditado(d.id);
                apagar_audio(&d.audio);
                self.pilula.avisar("Não ouvi nada", 1.6);
                return;
            }
            d.estado = "falhou".into();
            if let Err(e) = self.integracao.marcar_falha(&d) { nota(format!("ditado {}: não gravou a falha: {e}", d.id)); }
            self.pilula.avisar(if d.audio.is_empty() { "Não consegui transcrever" } else { "Não transcrevi. O áudio ficou guardado" }, 3.5);
            return;
        }
        d.estado = if r.falhas > 0 { "parcial" } else { "concluido" }.into();
        if let Err(e) = self.integracao.concluir_ditado(&d, &r) { nota(format!("ditado {}: resultado não foi gravado: {e}", d.id)); }
        nota(format!("ditado {}: {} palavras, {:.1} s, espera {:.2} s, limpador {}, recusas {}, falhas {}", d.id, r.palavras, d.duracao, d.latencia, r.limpador, r.recusas, r.falhas));
        if !r.trocas.is_empty() { self.mudou("dicionario"); }
        if !colar { self.pilula.avisar("Ditado guardado no histórico, sem colar", 3.0); return; }
        let texto = if self.prefs.espaco_no_fim() && !r.texto.ends_with('\n') { format!("{} ", r.texto) } else { r.texto.clone() };
        self.colar(&texto, r.falhas > 0);
    }

    fn colar(&self, texto: &str, parcial: bool) {
        match nativo::colagem::inserir(texto) {
            Insercao::Colado => {
                if parcial { self.pilula.avisar("Parte do áudio não foi transcrita", 3.5) } else { self.pilula.mostrar(FasePilula::Ociosa) }
            }
            Insercao::SoCopiado => self.pilula.avisar("Copiado. Cole com Ctrl+V", 3.0),
        }
    }

    pub fn cancelar(&self) {
        let gravacao = {
            let mut i = self.interno.lock().unwrap();
            if i.fase != Fase::Gravando { return; }
            i.fase = Fase::Parado;
            i.maos_livres = false;
            i.gravacao.take()
        };
        if let Some(mut g) = gravacao {
            g.captura.parar();
            let d = g.ditado;
            let integracao = self.integracao.clone();
            thread::spawn(move || {
                if let Ok((s, _)) = g.bomba.join() { s.cancelar(); }
                let _ = integracao.descartar_ditado(d.id);
                apagar_audio(&d.audio);
            });
        }
        self.pilula.mostrar(FasePilula::Ociosa);
        self.mudou("estado");
    }

    pub fn colar_ultimo(&self) {
        if self.fase() != Fase::Parado { return; }
        match self.integracao.ultimo_texto() {
            Ok(Some(t)) if !t.is_empty() => { if nativo::colagem::inserir(&t) == Insercao::SoCopiado { self.pilula.avisar("Copiado. Cole com Ctrl+V", 3.0); } }
            _ => self.pilula.avisar("Nenhum ditado para colar", 1.6),
        }
    }

    /// Passa de novo pelo reconhecedor o áudio guardado de um ditado que falhou ou ficou parcial.
    pub fn retranscrever(self: &Arc<Self>, id: i64) -> Result<(), String> {
        {
            let mut i = self.interno.lock().unwrap();
            if i.fase != Fase::Parado { return Err("Termine o ditado atual antes".into()); }
            i.fase = Fase::Processando;
        }
        self.pilula.mostrar(FasePilula::Processando);
        self.mudou("estado");
        let c = self.clone();
        thread::spawn(move || {
            let aviso = c.integracao.retranscrever(id, &caminhos::audios(), c.prefs.limpeza_ia()).unwrap_or_else(|e| { nota(format!("retranscrever {id}: {e}")); "Ainda não consegui transcrever" });
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
        if let Some(mut g) = g {
            g.captura.parar();
            let (tx, rx) = mpsc::channel();
            let bomba = g.bomba;
            thread::spawn(move || { let _ = tx.send(bomba.join().map(|(_, n)| n)); });
            match rx.recv_timeout(limite.saturating_duration_since(Instant::now())) {
                Ok(Ok(n)) => nota(format!("ditado {}: saída com {:.1} s gravados; volta pela recuperação", g.ditado.id, n as f64 / crate::audio::TAXA as f64)),
                _ => nota(format!("ditado {}: a gravação não terminou no prazo de saída", g.ditado.id)),
            }
            let mut i = self.interno.lock().unwrap();
            i.fase = Fase::Parado;
            i.maos_livres = false;
        }
        while self.fase() == Fase::Processando && Instant::now() < limite { thread::sleep(Duration::from_millis(10)); }
    }

    fn limpar_audios_antigos(&self) {
        let dias = self.prefs.guardar_audio_dias() as u64;
        let limite = Duration::from_secs(dias.max(1) * 86_400);
        let Ok(lista) = std::fs::read_dir(caminhos::audios()) else { return };
        for e in lista.flatten() {
            let velho = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).map(|i| i > limite).unwrap_or(false);
            if velho && e.path().extension().map(|x| x == "wav").unwrap_or(false) { let _ = std::fs::remove_file(e.path()); }
        }
    }
}

fn apagar_audio(nome: &str) {
    if nome.is_empty() || nome.contains(['/', '\\']) { return; }
    let _ = std::fs::remove_file(caminhos::audios().join(nome));
}

/// Liga a máquina do atalho ao controlador sem ciclo de referência.
struct Ponte(Weak<Controlador>);

impl Acoes for Ponte {
    fn comecar(&mut self) -> bool { self.0.upgrade().map(|c| c.comecar(false)).unwrap_or(false) }
    fn terminar(&mut self) { if let Some(c) = self.0.upgrade() { c.terminar(true) } }
    fn cancelar(&mut self) { if let Some(c) = self.0.upgrade() { c.cancelar() } }
    fn maos_livres(&mut self) { if let Some(c) = self.0.upgrade() { c.entrar_em_maos_livres() } }
    fn colar_ultimo(&mut self) { if let Some(c) = self.0.upgrade() { c.colar_ultimo() } }
    fn interromper(&mut self) { if let Some(c) = self.0.upgrade() { c.terminar(false) } }
}
