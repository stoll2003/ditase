//! Ilha no centro da borda superior do Windows (v1.0.2, SPEC-dynamic-island.md, com o ajuste de
//! aceite do Founder: base em curva contínua). Segunda janela Tauri transparente, sempre no topo,
//! fora da barra de tarefas e sem nunca roubar o foco (`WS_EX_NOACTIVATE`, posicionamento e
//! exibição só por `SetWindowPos` com `SWP_NOACTIVATE`, nunca `WebviewWindow::show()`).
//!
//! Contorno único: meia elipse com o topo reto rente à borda da tela (centro em `(w/2, 0)`,
//! semieixos `w/2` e `h`). É o mesmo contorno no desenho (`pilula.html`), no contorno de progresso e
//! no recorte nativo (`CreateEllipticRgn` + `SetWindowRgn`), para que o clique fora da curva atravesse
//! para a janela de baixo.
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, OnceLock, Weak,
    },
    time::{Duration, Instant},
};

use serde_json::{json, Value};
use tauri::{ipc::Channel, AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

pub const ROTULO: &str = "pilula";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fase {
    Ociosa,
    Gravando,
    MaosLivres,
    Processando,
}

impl Fase {
    pub fn id(self) -> &'static str {
        match self {
            Fase::Ociosa => "ociosa",
            Fase::Gravando => "gravando",
            Fase::MaosLivres => "maosLivres",
            Fase::Processando => "processando",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NivelAviso {
    Info,
    Erro,
}

impl NivelAviso {
    pub fn id(self) -> &'static str {
        match self {
            NivelAviso::Info => "info",
            NivelAviso::Erro => "erro",
        }
    }
}

/// Modo da janela da ilha. A largura exata de cada estado vem do `pilula.js`
/// (`modo:{nome}:{w}:{h}`); estes são os tamanhos usados enquanto ele não informa.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModoJanela {
    Repouso,
    Compacta,
    Aberta,
}

/// Limites de sanidade para a geometria pedida pela página (px lógicos).
const LARGURA_MIN: f64 = 40.0;
const LARGURA_MAX: f64 = 340.0;
const ALTURA_MIN: f64 = 4.0;
const ALTURA_MAX: f64 = 56.0;

impl ModoJanela {
    pub fn dimensoes_padrao(self) -> (f64, f64) {
        match self {
            ModoJanela::Repouso => (92.0, 7.0),
            ModoJanela::Compacta => (116.0, 40.0),
            ModoJanela::Aberta => (248.0, 54.0),
        }
    }

    pub fn de_nome(s: &str) -> Option<Self> {
        match s {
            "repouso" => Some(Self::Repouso),
            "compacta" => Some(Self::Compacta),
            "aberta" => Some(Self::Aberta),
            _ => None,
        }
    }
}

/// Interpreta o pedido de modo da página: `aberta` ou `aberta:248:48` (px lógicos).
/// Medidas fora dos limites são recusadas, para a página nunca pedir janela absurda.
pub fn interpretar_modo(s: &str) -> Option<(ModoJanela, Option<(f64, f64)>)> {
    let mut partes = s.split(':');
    let modo = ModoJanela::de_nome(partes.next()?)?;
    let (w, h) = match (partes.next(), partes.next(), partes.next()) {
        (None, None, None) => return Some((modo, None)),
        (Some(w), Some(h), None) => (w.parse::<f64>().ok()?, h.parse::<f64>().ok()?),
        _ => return None,
    };
    let valido = w.is_finite()
        && h.is_finite()
        && (LARGURA_MIN..=LARGURA_MAX).contains(&w)
        && (ALTURA_MIN..=ALTURA_MAX).contains(&h);
    valido.then_some((modo, Some((w, h))))
}

/// Dados geométricos de um monitor em pixels físicos (para testes determinísticos sem janela).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RetanguloMonitor {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub work_x: i32,
    pub work_y: i32,
    pub work_w: i32,
    pub work_h: i32,
    pub escala: f64,
    pub principal: bool,
}

impl RetanguloMonitor {
    fn mesmo(&self, outro: &RetanguloMonitor) -> bool {
        self.x == outro.x && self.y == outro.y && self.w == outro.w && self.h == outro.h
    }
}

/// Posição e tamanho da janela da ilha em pixels físicos.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PosicaoIlha {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

fn posicao_no_monitor(m: &RetanguloMonitor, w_log: f64, h_log: f64) -> PosicaoIlha {
    let w = (w_log * m.escala).round() as i32;
    let h = (h_log * m.escala).round() as i32;
    let x = m.work_x + ((m.work_w - w) as f64 / 2.0).round() as i32;
    // Topo rente: sem folga de 1 px, também em escalas fracionárias.
    PosicaoIlha {
        x,
        y: m.work_y,
        w,
        h,
    }
}

/// Escolhe monitor e coordenadas físicas da ilha (spec §6.4):
/// 1. Centro da borda superior da área de trabalho do monitor principal.
/// 2. Se outro monitor encosta acima do principal na faixa horizontal da ilha (monitores
///    empilhados), usa o monitor mais alto dessa coluna, para que `y = work_y` seja sempre uma
///    parede física onde o ponteiro para.
/// 3. Barra de tarefas no topo: `work_y > y`, e a ilha encosta logo abaixo dela.
pub fn escolher_topo(
    monitores: &[RetanguloMonitor],
    w_log: f64,
    h_log: f64,
) -> Option<PosicaoIlha> {
    let principal = monitores
        .iter()
        .find(|m| m.principal)
        .or_else(|| monitores.first())
        .copied()?;
    let no_principal = posicao_no_monitor(&principal, w_log, h_log);
    let faixa = (no_principal.x, no_principal.x + no_principal.w);
    let sobrepoe = |m: &RetanguloMonitor| m.x < faixa.1 && m.x + m.w > faixa.0;

    let encosta_acima = monitores
        .iter()
        .any(|m| !m.mesmo(&principal) && (m.y + m.h - principal.y).abs() <= 4 && sobrepoe(m));
    if !encosta_acima {
        return Some(no_principal);
    }
    let mais_alto = monitores
        .iter()
        .filter(|m| sobrepoe(m))
        .min_by_key(|m| m.work_y)
        .copied()
        .unwrap_or(principal);
    Some(posicao_no_monitor(&mais_alto, w_log, h_log))
}

/// Retângulo envolvente, em pixels físicos relativos à janela, da elipse que vira a região de
/// recorte (`CreateEllipticRgn`): centro em `(w/2, 0)`, semieixos `w/2` e `h`. Só a metade de
/// baixo cai dentro da janela, e o topo fica reto na borda da tela. O `+ 1` compensa a regra do GDI
/// de excluir a borda direita e a de baixo do retângulo.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn regiao_meia_elipse(w: i32, h: i32) -> (i32, i32, i32, i32) {
    (0, -h, w + 1, h + 1)
}

/// Teste de ponto do mesmo contorno (centro do pixel `(px, py)`), espelhando `regiao_meia_elipse`
/// e o desenho em `pilula.html`. Usado nos testes de recorte e de encaixe dos controles.
#[cfg(test)]
pub fn dentro_da_meia_elipse(w: f64, h: f64, px: f64, py: f64) -> bool {
    if w <= 0.0 || h <= 0.0 || py < 0.0 || py > h {
        return false;
    }
    let a = w / 2.0;
    let dx = (px - a) / a;
    let dy = py / h;
    dx * dx + dy * dy <= 1.0
}

/// Retângulo físico `(x, y, w, h)`.
#[cfg_attr(not(windows), allow(dead_code))]
pub type Retangulo = (i32, i32, i32, i32);

/// O que fazer quando a tela muda com o app aberto.
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Conserto {
    Nada,
    /// Posição ou tamanho fora do certo: reaplica `SetWindowPos` + recorte.
    Reaplicar,
    /// A janela ficou presa num DPI diferente do monitor (o WebView2 desenharia na escala errada):
    /// recria a janela, que nasce com o DPI atual.
    Recriar,
}

/// Decide o conserto comparando o DPI da janela com o do monitor e os retângulos pedido, real e
/// calculado para a tela de agora. DPI `0` significa leitura indisponível e não decide nada.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn decidir_conserto(
    dpi_janela: u32,
    dpi_monitor: u32,
    esperado: Option<Retangulo>,
    real: Option<Retangulo>,
    calculado: Option<Retangulo>,
) -> Conserto {
    if dpi_janela != 0 && dpi_monitor != 0 && dpi_janela != dpi_monitor {
        return Conserto::Recriar;
    }
    match (calculado, esperado) {
        (Some(c), Some(e)) if c != e || real.is_some_and(|r| r != e) => Conserto::Reaplicar,
        (Some(_), None) => Conserto::Reaplicar,
        _ => Conserto::Nada,
    }
}

/// No máximo `max` ações por `janela` de tempo: impede laço se outro código insistir em mover a ilha.
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Debug)]
pub struct Limite {
    janela: Duration,
    max: usize,
    marcas: VecDeque<Instant>,
}

#[cfg_attr(not(windows), allow(dead_code))]
impl Limite {
    pub fn novo(janela: Duration, max: usize) -> Self {
        Self {
            janela,
            max,
            marcas: VecDeque::new(),
        }
    }

    pub fn permitir(&mut self, agora: Instant) -> bool {
        while self
            .marcas
            .front()
            .is_some_and(|t| agora.duration_since(*t) >= self.janela)
        {
            self.marcas.pop_front();
        }
        if self.marcas.len() >= self.max {
            return false;
        }
        self.marcas.push_back(agora);
        true
    }
}

/// Atrasos da conferência depois de um aviso de tela: o primeiro roda depois do `SetWindowPos`
/// que o tao faz ao fim do `WM_DPICHANGED`; o segundo pega a área de trabalho atualizada tarde.
pub const ATRASOS_CONFERENCIA: [Duration; 2] =
    [Duration::from_millis(120), Duration::from_millis(700)];

/// Construtores puros das mensagens do protocolo da aba (testáveis sem janela).
pub fn msg_fase(fase: Fase) -> Value {
    json!({"tipo": "fase", "fase": fase.id()})
}

pub fn msg_aviso(texto: &str, segundos: f64, nivel: NivelAviso) -> Value {
    let duracao = match nivel {
        NivelAviso::Erro => segundos.max(3.5),
        NivelAviso::Info => segundos,
    };
    json!({"tipo": "aviso", "texto": texto, "segundos": duracao, "nivel": nivel.id()})
}

pub fn msg_modelo(feito: Option<u64>, total: u64) -> Value {
    json!({"tipo": "modelo", "feito": feito, "total": total})
}

struct Interno {
    fase: Fase,
    modo: ModoJanela,
    sempre_visivel: bool,
    tela_cheia: bool,
    baixando_modelo: Option<(u64, u64)>,
    progresso_ativo: bool,
    aviso_ate: Option<Instant>,
    /// Tamanho lógico atual da ilha (largura exata do estado informada pela página).
    geometria: (f64, f64),
    ultimo_nivel: Instant,
    atalho: String,
    canal: Option<Channel<Value>>,
    /// Geração ativa do vigia de cursor (`0` = desligado). Cada abertura recebe um id próprio
    /// para que uma reabertura rápida invalide o vigia anterior antes do próximo ciclo.
    vigia_geracao_ativa: Arc<AtomicU64>,
    vigia_proxima_geracao: u64,
    #[cfg_attr(not(windows), allow(dead_code))]
    limite_reaplicar: Limite,
    #[cfg_attr(not(windows), allow(dead_code))]
    limite_recriar: Limite,
}

fn modo_da_fase(i: &Interno) -> ModoJanela {
    match i.fase {
        Fase::Ociosa => modo_ocioso_padrao(i),
        Fase::Gravando | Fase::Processando => ModoJanela::Compacta,
        Fase::MaosLivres => ModoJanela::Aberta,
    }
}

/// Atualiza `modo` e a geração do vigia de cursor num único lugar (I-3):
/// fora de `Aberta` o vigia é sempre desligado (`0`); ao entrar em `Aberta` (quando ainda não
/// havia vigia ativo), emite `(geracao_ativa, minha_geracao)` para a thread do vigia.
fn atualizar_modo_e_vigia(i: &mut Interno, modo: ModoJanela) -> Option<(Arc<AtomicU64>, u64)> {
    if i.modo != modo {
        // Troca de modo comandada pelo Rust: usa o tamanho padrão até a página informar o exato.
        i.geometria = modo.dimensoes_padrao();
    }
    i.modo = modo;
    if modo == ModoJanela::Aberta {
        if i.vigia_geracao_ativa.load(Ordering::Acquire) == 0 {
            i.vigia_proxima_geracao = i.vigia_proxima_geracao.wrapping_add(1).max(1);
            let geracao = i.vigia_proxima_geracao;
            i.vigia_geracao_ativa.store(geracao, Ordering::Release);
            Some((i.vigia_geracao_ativa.clone(), geracao))
        } else {
            None
        }
    } else {
        i.vigia_geracao_ativa.store(0, Ordering::Release);
        None
    }
}

fn vigia_ainda_valido(geracao_ativa: &AtomicU64, minha_geracao: u64) -> bool {
    minha_geracao != 0 && geracao_ativa.load(Ordering::Acquire) == minha_geracao
}

/// Regra do timer de segurança de `Pilula::avisar` (I-4): só encolhe pelo Rust se não houver
/// canal IPC conectado ou se o ponteiro já estiver fora da janela (com hover ativo, o `pilula.js`
/// segura o aviso e manda `modo:repouso` quando o ponteiro sai).
fn deve_encolher_aviso_no_timer(canal_conectado: bool, cursor_fora: bool) -> bool {
    !canal_conectado || cursor_fora
}

fn modo_ocioso_padrao(i: &Interno) -> ModoJanela {
    if i.baixando_modelo.is_some() || i.progresso_ativo {
        ModoJanela::Compacta
    } else {
        ModoJanela::Repouso
    }
}

/// Ao receber atualização de `modelo` ou `progresso` em estado ocioso, preserva `Aberta` se a aba
/// estiver aberta pelo ponteiro; caso contrário, usa `Compacta` em atividade ou `Repouso`.
fn modo_para_atividade_ociosa(i: &Interno) -> ModoJanela {
    if i.modo == ModoJanela::Aberta {
        ModoJanela::Aberta
    } else {
        modo_ocioso_padrao(i)
    }
}

pub struct Pilula {
    app: AppHandle,
    interno: Mutex<Interno>,
    eu: OnceLock<Weak<Pilula>>,
    recriando: AtomicBool,
    geracao_conferencia: AtomicU64,
}

/// Cria a janela da ilha: transparente, sem foco, fora da barra de tarefas, escondida até o
/// primeiro `SetWindowPos`. Também usada para recriá-la quando o DPI dela fica preso.
fn construir_janela(app: &AppHandle, eu: Weak<Pilula>) -> Option<WebviewWindow> {
    let (lw, lh) = ModoJanela::Repouso.dimensoes_padrao();
    let janela = WebviewWindowBuilder::new(app, ROTULO, WebviewUrl::App("pilula.html".into()))
        .title("Dita.se")
        .inner_size(lw, lh)
        .resizable(false)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .visible(false)
        .on_navigation(crate::navegacao_permitida)
        .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
        .build();
    let j = match janela {
        Ok(j) => j,
        Err(e) => {
            crate::caminhos::nota(format!("ilha: não abriu ({e})"));
            return None;
        }
    };
    #[cfg(windows)]
    if let Ok(h) = j.hwnd() {
        crate::nativo::janela::sem_foco(h.0 as isize);
        crate::nativo::janela::registrar_ilha(h.0 as isize);
    }
    // O tao trata o WM_DPICHANGED chamando este evento e, logo depois, faz o próprio SetWindowPos,
    // que sobrescreveria a ilha: por isso a conferência roda adiada, e não aqui dentro.
    j.on_window_event(move |ev| {
        if let tauri::WindowEvent::ScaleFactorChanged { .. } = ev {
            if let Some(p) = eu.upgrade() {
                p.reconferir("escala");
            }
        }
    });
    Some(j)
}

impl Pilula {
    pub fn criar(app: &AppHandle, sempre_visivel: bool, atalho: &str) -> Arc<Self> {
        let p = Arc::new(Self {
            app: app.clone(),
            interno: Mutex::new(Interno {
                fase: Fase::Ociosa,
                modo: ModoJanela::Repouso,
                sempre_visivel,
                tela_cheia: crate::nativo::janela::tela_cheia_ativa(),
                baixando_modelo: None,
                progresso_ativo: false,
                aviso_ate: None,
                geometria: ModoJanela::Repouso.dimensoes_padrao(),
                ultimo_nivel: Instant::now(),
                atalho: atalho.to_string(),
                canal: None,
                vigia_geracao_ativa: Arc::new(AtomicU64::new(0)),
                vigia_proxima_geracao: 0,
                limite_reaplicar: Limite::novo(Duration::from_secs(3), 8),
                limite_recriar: Limite::novo(Duration::from_secs(20), 2),
            }),
            eu: OnceLock::new(),
            recriando: AtomicBool::new(false),
            geracao_conferencia: AtomicU64::new(0),
        });
        let _ = p.eu.set(Arc::downgrade(&p));
        let _ = construir_janela(app, Arc::downgrade(&p));

        // Tela cheia (EVENT_SYSTEM_FOREGROUND) e mudanças de tela (WM_DISPLAYCHANGE,
        // WM_SETTINGCHANGE, deriva da posição): tudo por evento, zero consulta em repouso.
        let p_cheia = Arc::downgrade(&p);
        let p_tela = Arc::downgrade(&p);
        crate::nativo::janela::observar(
            Arc::new(move |ativa| {
                if let Some(p) = p_cheia.upgrade() {
                    p.definir_tela_cheia(ativa);
                }
            }),
            Arc::new(move |motivo| {
                if let Some(p) = p_tela.upgrade() {
                    p.reconferir(motivo);
                }
            }),
        );

        p.enviar(json!({"tipo": "atalho", "simbolo": atalho}));
        p.mostrar(Fase::Ociosa);
        // Abertura em DPI alto: o WM_DPICHANGED inicial do tao pode chegar depois do primeiro
        // posicionamento; a conferência adiada corrige.
        p.reconferir("abertura");
        p
    }

    /// Agenda a conferência da ilha depois de um aviso de tela. Avisos em rajada se juntam: só a
    /// conferência do aviso mais recente roda no primeiro atraso.
    pub fn reconferir(&self, motivo: &'static str) {
        let Some(eu) = self.eu.get().cloned() else {
            return;
        };
        let geracao = self.geracao_conferencia.fetch_add(1, Ordering::AcqRel) + 1;
        let _ = std::thread::Builder::new()
            .name("ditase.ilha.tela".into())
            .spawn(move || {
                for (n, atraso) in ATRASOS_CONFERENCIA.iter().enumerate() {
                    std::thread::sleep(*atraso);
                    let Some(p) = eu.upgrade() else {
                        return;
                    };
                    if n == 0 && p.geracao_conferencia.load(Ordering::Acquire) != geracao {
                        continue;
                    }
                    p.conferir(motivo);
                }
            });
    }

    #[cfg(not(windows))]
    fn conferir(&self, _motivo: &'static str) {}

    /// Compara a ilha com a tela de agora e conserta (reaplica ou recria), com limite contra laço.
    #[cfg(windows)]
    fn conferir(&self, motivo: &'static str) {
        if self.recriando.load(Ordering::Acquire) {
            return;
        }
        let Some(j) = self.janela() else {
            return;
        };
        let (visivel, (w_log, h_log)) = {
            let i = self.interno.lock().unwrap();
            (Self::deve_estar_visivel(&i), i.geometria)
        };
        if !visivel {
            return;
        }
        let Ok(h) = j.hwnd() else {
            return;
        };
        let hwnd = h.0 as isize;
        let monitores = coletar_monitores(&j);
        let calculado = escolher_topo(&monitores, w_log, h_log).map(|p| (p.x, p.y, p.w, p.h));
        let dpi_janela = crate::nativo::janela::dpi_da_janela(hwnd);
        let dpi_monitor = crate::nativo::janela::dpi_do_monitor(hwnd);
        let esperado = crate::nativo::janela::retangulo_esperado();
        let real = crate::nativo::janela::retangulo(hwnd);
        let conserto = decidir_conserto(dpi_janela, dpi_monitor, esperado, real, calculado);
        if conserto == Conserto::Nada && motivo != "abertura" {
            return;
        }
        let permitido = {
            let mut i = self.interno.lock().unwrap();
            match conserto {
                Conserto::Nada => true,
                Conserto::Reaplicar => i.limite_reaplicar.permitir(Instant::now()),
                Conserto::Recriar => i.limite_recriar.permitir(Instant::now()),
            }
        };
        let telas: Vec<String> = monitores
            .iter()
            .map(|m| {
                format!(
                    "{}x{}@{},{} area {},{} {}x{} escala {}{}",
                    m.w,
                    m.h,
                    m.x,
                    m.y,
                    m.work_x,
                    m.work_y,
                    m.work_w,
                    m.work_h,
                    m.escala,
                    if m.principal { " principal" } else { "" }
                )
            })
            .collect();
        crate::caminhos::nota(format!(
            "ilha: {motivo}: dpi janela {dpi_janela} monitor {dpi_monitor}; pedido {esperado:?} \
             real {real:?} calculado {calculado:?}; telas [{}] -> {conserto:?}{}",
            telas.join("; "),
            if permitido {
                ""
            } else {
                " (limite atingido, ignorado)"
            }
        ));
        if !permitido {
            return;
        }
        match conserto {
            Conserto::Nada => {}
            Conserto::Reaplicar => self.aplicar_janela(),
            Conserto::Recriar => self.recriar(),
        }
    }

    /// Recria a janela da ilha (o DPI de uma janela só é escolhido na criação ou num
    /// `WM_DPICHANGED`; se o Windows não mandou, a janela nova nasce com o DPI do monitor).
    #[cfg_attr(not(windows), allow(dead_code))]
    fn recriar(&self) {
        if self.recriando.swap(true, Ordering::AcqRel) {
            return;
        }
        let Some(eu) = self.eu.get().cloned() else {
            self.recriando.store(false, Ordering::Release);
            return;
        };
        let _ = std::thread::Builder::new()
            .name("ditase.ilha.recriar".into())
            .spawn(move || {
                let Some(p) = eu.upgrade() else {
                    return;
                };
                {
                    // A página nova volta ao repouso e reconecta o canal; o estado do ditado
                    // (fase, download, progresso) é reenviado em `conectar`.
                    let mut i = p.interno.lock().unwrap();
                    i.canal = None;
                    i.aviso_ate = None;
                    let modo = modo_da_fase(&i);
                    let _ = atualizar_modo_e_vigia(&mut i, modo);
                }
                if let Some(j) = p.janela() {
                    let _ = j.destroy();
                }
                for _ in 0..60 {
                    if p.janela().is_none() {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                let criada = construir_janela(&p.app, eu.clone()).is_some();
                crate::caminhos::nota(format!("ilha: janela recriada ({criada})"));
                p.recriando.store(false, Ordering::Release);
                p.aplicar_janela();
                p.reconferir("recriada");
            });
    }

    fn janela(&self) -> Option<WebviewWindow> {
        self.app.get_webview_window(ROTULO)
    }

    /// Canal exclusivo da pílula (M-3): sem `core:event:allow-listen`, a pílula não pode assinar
    /// eventos da janela principal (como `ditado:trechos`), e recebe só o que sai por aqui.
    fn enviar(&self, mensagem: Value) {
        let canal = self.interno.lock().unwrap().canal.clone();
        if let Some(c) = canal {
            let _ = c.send(mensagem);
        }
    }

    /// A página da ilha carregou e entregou o canal IPC dela: guarda e reenvia atalho, modelo e fase.
    pub fn conectar(&self, canal: Channel<Value>) {
        self.interno.lock().unwrap().canal = Some(canal);
        self.reenviar();
    }

    /// Reenvia o estado completo para a página da aba.
    pub fn reenviar(&self) {
        let (fase, atalho, baixando) = {
            let i = self.interno.lock().unwrap();
            (i.fase, i.atalho.clone(), i.baixando_modelo)
        };
        self.enviar(json!({"tipo": "atalho", "simbolo": atalho}));
        if let Some((feito, total)) = baixando {
            self.enviar(msg_modelo(Some(feito), total));
        }
        self.enviar(msg_fase(fase));
    }

    fn deve_estar_visivel(i: &Interno) -> bool {
        let aviso_ativo = i.aviso_ate.is_some_and(|ate| Instant::now() < ate);
        let em_atividade = i.fase != Fase::Ociosa
            || aviso_ativo
            || i.baixando_modelo.is_some()
            || i.progresso_ativo;
        if em_atividade {
            true
        } else {
            i.sempre_visivel && !i.tela_cheia
        }
    }

    /// Posiciona, redimensiona, recorta e mostra/esconde a janela sem nunca ativar
    /// (`SWP_NOACTIVATE`). Recalculado a cada troca de estado e de escala do monitor.
    pub fn aplicar_janela(&self) {
        let Some(j) = self.janela() else {
            return;
        };
        let (visivel, (w_log, h_log)) = {
            let i = self.interno.lock().unwrap();
            (Self::deve_estar_visivel(&i), i.geometria)
        };
        if !visivel {
            #[cfg(windows)]
            if let Ok(h) = j.hwnd() {
                crate::nativo::janela::esconder(h.0 as isize);
            }
            #[cfg(not(windows))]
            let _ = j.hide();
            return;
        }
        let monitores = coletar_monitores(&j);
        let Some(pos) = escolher_topo(&monitores, w_log, h_log) else {
            return;
        };
        #[cfg(windows)]
        if let Ok(h) = j.hwnd() {
            let hwnd = h.0 as isize;
            crate::nativo::janela::posicionar_sem_ativar(hwnd, pos.x, pos.y, pos.w, pos.h, true);
            let (l, t, r, b) = regiao_meia_elipse(pos.w, pos.h);
            crate::nativo::janela::recortar_elipse(hwnd, l, t, r, b);
        }
        #[cfg(not(windows))]
        let _ = pos;
    }

    fn iniciar_vigia_se_preciso(&self, vigia: Option<(Arc<AtomicU64>, u64)>) {
        let Some((geracao_ativa, minha_geracao)) = vigia else {
            return;
        };
        // Fallback de saída do ponteiro (MOTION-windows.md §6): 10 Hz só enquanto aberta, nunca em repouso.
        let app = self.app.clone();
        let _ = std::thread::Builder::new()
            .name("ditase.aba.cursor".into())
            .spawn(move || {
                while vigia_ainda_valido(&geracao_ativa, minha_geracao) {
                    std::thread::sleep(Duration::from_millis(100));
                    if !vigia_ainda_valido(&geracao_ativa, minha_geracao) {
                        break;
                    }
                    let Some(j) = app.get_webview_window(ROTULO) else {
                        break;
                    };
                    #[cfg(windows)]
                    if let Ok(h) = j.hwnd() {
                        if crate::nativo::janela::cursor_fora_da_janela(h.0 as isize) {
                            if let Some(casca) = app.try_state::<Arc<crate::Casca>>() {
                                casca.pilula.enviar(json!({"tipo": "ponteiroFora"}));
                            }
                        }
                    }
                    #[cfg(not(windows))]
                    let _ = j;
                }
            });
    }

    /// Pedido da página (`modo:{nome}[:{w}:{h}]`): troca o modo e, se vier, usa a geometria exata
    /// do estado. A página só pede para encolher depois que a animação de saída terminou.
    pub fn definir_modo(&self, modo: ModoJanela, geometria: Option<(f64, f64)>) {
        let iniciar = {
            let mut i = self.interno.lock().unwrap();
            if modo != ModoJanela::Aberta {
                // Se o JS comandou o fechamento (ex.: fim de aviso após sair o hover), limpa aviso_ate.
                i.aviso_ate = None;
            }
            let antes = (i.modo, i.geometria);
            let flag = atualizar_modo_e_vigia(&mut i, modo);
            if let Some(g) = geometria {
                i.geometria = g;
            }
            if antes == (i.modo, i.geometria) && flag.is_none() {
                return;
            }
            flag
        };
        self.aplicar_janela();
        self.iniciar_vigia_se_preciso(iniciar);
    }

    pub fn definir_tela_cheia(&self, ativa: bool) {
        {
            let mut i = self.interno.lock().unwrap();
            if i.tela_cheia == ativa {
                return;
            }
            i.tela_cheia = ativa;
        }
        self.aplicar_janela();
    }

    pub fn fase(&self) -> Fase {
        self.interno.lock().unwrap().fase
    }

    pub fn mostrar(&self, fase: Fase) {
        let iniciar = {
            let mut i = self.interno.lock().unwrap();
            i.fase = fase;
            if fase != Fase::Ociosa {
                i.aviso_ate = None;
            }
            let modo = modo_da_fase(&i);
            atualizar_modo_e_vigia(&mut i, modo)
        };
        self.aplicar_janela();
        self.iniciar_vigia_se_preciso(iniciar);
        self.enviar(msg_fase(fase));
    }

    /// Mensagem curta com nível (`info` ou `erro`); a aba volta sozinha a ociosa depois de `segundos`.
    pub fn avisar(&self, texto: &str, segundos: f64, nivel: NivelAviso) {
        let duracao = match nivel {
            NivelAviso::Erro => segundos.max(3.5),
            NivelAviso::Info => segundos,
        };
        let iniciar = {
            let mut i = self.interno.lock().unwrap();
            i.fase = Fase::Ociosa;
            i.aviso_ate = Some(Instant::now() + Duration::from_secs_f64(duracao + 0.4));
            atualizar_modo_e_vigia(&mut i, ModoJanela::Aberta)
        };
        self.aplicar_janela();
        self.iniciar_vigia_se_preciso(iniciar);
        self.enviar(msg_aviso(texto, duracao, nivel));
        if nivel == NivelAviso::Erro {
            if let Some(casca) = self.app.try_state::<Arc<crate::Casca>>() {
                if let Some(b) = casca.bandeja.get() {
                    b.registrar_erro(texto);
                }
            }
        }

        // Guarda de segurança (I-4): o fechamento normal é comandado por `pilula.js` (que segura
        // o aviso enquanto o ponteiro está sobre a aba). Este timer só atua se não houver canal
        // conectado ou se o ponteiro estiver fora da janela.
        let app = self.app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs_f64(duracao + 0.5));
            let Some(estado) = app.try_state::<Arc<crate::Casca>>() else {
                return;
            };
            let cursor_fora = {
                #[cfg(windows)]
                {
                    estado
                        .pilula
                        .janela()
                        .and_then(|j| j.hwnd().ok())
                        .is_none_or(|h| crate::nativo::janela::cursor_fora_da_janela(h.0 as isize))
                }
                #[cfg(not(windows))]
                {
                    true
                }
            };
            let mut i = estado.pilula.interno.lock().unwrap();
            let canal_conectado = i.canal.is_some();
            if i.aviso_ate.is_some_and(|ate| Instant::now() >= ate)
                && deve_encolher_aviso_no_timer(canal_conectado, cursor_fora)
            {
                i.aviso_ate = None;
                if i.fase == Fase::Ociosa {
                    let modo = modo_ocioso_padrao(&i);
                    let _ = atualizar_modo_e_vigia(&mut i, modo);
                }
                drop(i);
                estado.pilula.aplicar_janela();
            }
        });
    }

    /// Progresso do download do modelo de voz (`feito: None` encerra o estado de download).
    pub fn modelo(&self, feito: Option<u64>, total: u64) {
        let iniciar = {
            let mut i = self.interno.lock().unwrap();
            i.baixando_modelo = feito.map(|f| (f, total));
            if i.fase == Fase::Ociosa && i.aviso_ate.is_none() {
                let modo = modo_para_atividade_ociosa(&i);
                atualizar_modo_e_vigia(&mut i, modo)
            } else {
                None
            }
        };
        self.aplicar_janela();
        self.iniciar_vigia_se_preciso(iniciar);
        self.enviar(msg_modelo(feito, total));
    }

    /// Nível do microfone, no máximo a cada 50 ms (como no Mac).
    pub fn empurrar_nivel(&self, nivel: f32) {
        {
            let mut i = self.interno.lock().unwrap();
            if i.ultimo_nivel.elapsed() < Duration::from_millis(50) {
                return;
            }
            i.ultimo_nivel = Instant::now();
        }
        self.enviar(json!({"tipo": "nivel", "nivel": nivel}));
    }

    pub fn segundos(&self, s: u64) {
        self.enviar(json!({"tipo": "segundos", "segundos": s}));
    }

    pub fn atalho(&self, simbolo: &str) {
        self.interno.lock().unwrap().atalho = simbolo.to_string();
        self.enviar(json!({"tipo": "atalho", "simbolo": simbolo}));
    }

    pub fn sempre_visivel(&self, sim: bool) {
        self.interno.lock().unwrap().sempre_visivel = sim;
        self.aplicar_janela();
    }

    /// Progresso de arquivo ou reunião (`ProgressoDaPilula`); `None` encerra.
    pub fn progresso(&self, evento: Option<Value>) {
        let iniciar = {
            let mut i = self.interno.lock().unwrap();
            i.progresso_ativo = evento.is_some();
            if i.fase == Fase::Ociosa && i.aviso_ate.is_none() {
                let modo = modo_para_atividade_ociosa(&i);
                atualizar_modo_e_vigia(&mut i, modo)
            } else {
                None
            }
        };
        self.aplicar_janela();
        self.iniciar_vigia_se_preciso(iniciar);
        self.enviar(json!({"tipo": "progresso", "progresso": evento}));
    }
}

fn coletar_monitores(j: &WebviewWindow) -> Vec<RetanguloMonitor> {
    let principal = j.primary_monitor().ok().flatten();
    let lista = j.available_monitors().unwrap_or_default();
    let converter = |m: &tauri::Monitor, eh_principal: bool| {
        let pos = m.position();
        let tam = m.size();
        let work = m.work_area();
        RetanguloMonitor {
            x: pos.x,
            y: pos.y,
            w: tam.width as i32,
            h: tam.height as i32,
            work_x: work.position.x,
            work_y: work.position.y,
            work_w: work.size.width as i32,
            work_h: work.size.height as i32,
            escala: m.scale_factor(),
            principal: eh_principal,
        }
    };
    if lista.is_empty() {
        return principal
            .as_ref()
            .map(|m| vec![converter(m, true)])
            .unwrap_or_default();
    }
    lista
        .iter()
        .map(|m| {
            let eh_principal = principal
                .as_ref()
                .is_some_and(|p| p.position() == m.position() && p.size() == m.size());
            converter(m, eh_principal)
        })
        .collect()
}

#[cfg(test)]
mod testes {
    use super::*;

    fn monitor(x: i32, y: i32, w: i32, h: i32, escala: f64, principal: bool) -> RetanguloMonitor {
        RetanguloMonitor {
            x,
            y,
            w,
            h,
            work_x: x,
            work_y: y,
            work_w: w,
            work_h: h - 48,
            escala,
            principal,
        }
    }

    #[test]
    fn protocolo_aviso_leva_nivel_e_minimo_de_erro() {
        let info = msg_aviso("Copiado. Cole com Ctrl+V", 3.0, NivelAviso::Info);
        assert_eq!(info["tipo"], "aviso");
        assert_eq!(info["nivel"], "info");
        assert_eq!(info["segundos"], 3.0);

        let erro = msg_aviso("Microfone indisponível", 2.0, NivelAviso::Erro);
        assert_eq!(erro["tipo"], "aviso");
        assert_eq!(erro["nivel"], "erro");
        assert_eq!(erro["segundos"], 3.5);
    }

    #[test]
    fn protocolo_modelo_serializa_feito_total_e_null() {
        let baixando = msg_modelo(Some(420), 1000);
        assert_eq!(
            baixando,
            json!({"tipo": "modelo", "feito": 420, "total": 1000})
        );

        let fim = msg_modelo(None, 0);
        assert_eq!(fim, json!({"tipo": "modelo", "feito": null, "total": 0}));
    }

    #[test]
    fn monitor_unico_centraliza_no_topo_em_varias_escalas() {
        for escala in [1.0, 1.25, 1.5, 1.75, 2.0, 2.5] {
            let m = monitor(0, 0, 3840, 2160, escala, true);
            for (w_log, h_log) in [
                (92.0, 7.0),
                (112.0, 11.0),
                (116.0, 40.0),
                (248.0, 54.0),
                (336.0, 56.0),
            ] {
                let pos = escolher_topo(&[m], w_log, h_log).unwrap();
                assert_eq!(pos.w, (w_log * escala).round() as i32);
                assert_eq!(pos.h, (h_log * escala).round() as i32);
                // Topo rente, sem folga de 1 px.
                assert_eq!(pos.y, m.work_y);
                // Centro: as sobras dos dois lados diferem no máximo 1 px físico.
                let esquerda = pos.x - m.work_x;
                let direita = m.work_x + m.work_w - (pos.x + pos.w);
                assert!(
                    (esquerda - direita).abs() <= 1,
                    "escala {escala}: {esquerda} x {direita}"
                );
            }
        }
    }

    #[test]
    fn barra_de_tarefas_no_topo_encosta_logo_abaixo_dela() {
        let mut principal = monitor(0, 0, 1920, 1080, 1.0, true);
        principal.work_y = 48;
        principal.work_h = 1032;
        let pos = escolher_topo(&[principal], 248.0, 54.0).unwrap();
        assert_eq!(pos.y, 48);
        assert_eq!(pos.x, (1920 - 248) / 2);
    }

    #[test]
    fn monitores_lado_a_lado_usam_o_principal() {
        let esquerda = monitor(-2560, -200, 2560, 1440, 1.5, false);
        let principal = monitor(0, 0, 1920, 1080, 1.0, true);
        let direita = monitor(1920, 0, 1920, 1080, 1.0, false);
        let pos = escolher_topo(&[esquerda, principal, direita], 116.0, 40.0).unwrap();
        assert_eq!((pos.x, pos.y, pos.w, pos.h), ((1920 - 116) / 2, 0, 116, 40));
    }

    #[test]
    fn monitores_empilhados_usam_o_mais_alto_com_a_escala_dele() {
        let principal = monitor(0, 0, 1920, 1080, 1.0, true);
        let acima = monitor(-320, -1440, 2560, 1440, 1.5, false);
        let pos = escolher_topo(&[principal, acima], 248.0, 54.0).unwrap();
        assert_eq!(pos.y, -1440);
        assert_eq!(pos.w, 372);
        assert_eq!(pos.h, 81);
        assert_eq!(pos.x, -320 + (2560 - 372) / 2);
    }

    #[test]
    fn monitor_acima_fora_da_faixa_da_ilha_mantem_o_principal() {
        let principal = monitor(0, 0, 1920, 1080, 1.0, true);
        // Encosta acima, mas só sobre o canto esquerdo, longe do centro onde fica a ilha.
        let acima_canto = monitor(-1500, -1080, 1920, 1080, 1.0, false);
        let pos = escolher_topo(&[principal, acima_canto], 248.0, 54.0).unwrap();
        assert_eq!(pos.y, 0);
        assert_eq!(pos.x, (1920 - 248) / 2);
    }

    #[test]
    fn sem_monitores_nao_posiciona() {
        assert!(escolher_topo(&[], 92.0, 7.0).is_none());
    }

    #[test]
    fn regiao_e_meia_elipse_com_topo_reto_e_cantos_vazados() {
        for escala in [1.0_f64, 1.25, 1.5, 2.0] {
            for (w_log, h_log) in [(92.0, 7.0), (116.0, 40.0), (248.0, 54.0), (336.0, 56.0)] {
                let w = (w_log * escala).round() as i32;
                let h = (h_log * escala).round() as i32;
                let (l, t, r, b) = regiao_meia_elipse(w, h);
                // Elipse centrada na borda de cima da janela: só a metade de baixo fica dentro.
                assert_eq!((l + r - 1) as f64 / 2.0, w as f64 / 2.0);
                assert_eq!(t + b - 1, 0);
                let (wf, hf) = (w as f64, h as f64);
                // Topo: rente à borda em quase toda a largura.
                assert!(dentro_da_meia_elipse(wf, hf, wf * 0.05, 0.5));
                assert!(dentro_da_meia_elipse(wf, hf, wf * 0.95, 0.5));
                // Parte mais baixa no centro.
                assert!(dentro_da_meia_elipse(wf, hf, wf / 2.0, hf - 0.5));
                // Cantos de baixo ficam fora: o clique ali atravessa para a janela de baixo.
                for (px, py) in [(0.5, hf - 0.5), (wf - 0.5, hf - 0.5)] {
                    assert!(
                        !dentro_da_meia_elipse(wf, hf, px, py),
                        "{w}x{h} ({px},{py})"
                    );
                }
                // Curva contínua e simétrica: a altura da borda cai monotonicamente do centro às laterais.
                let mut anterior = f64::MAX;
                for k in 0..=20 {
                    let px = wf / 2.0 + (wf / 2.0) * (k as f64 / 20.0) * 0.999;
                    let a = wf / 2.0;
                    let altura = hf * (1.0_f64 - ((px - a) / a).powi(2)).max(0.0).sqrt();
                    assert!(altura <= anterior);
                    anterior = altura;
                    let espelho = wf - px;
                    assert_eq!(
                        dentro_da_meia_elipse(wf, hf, px, altura * 0.9),
                        dentro_da_meia_elipse(wf, hf, espelho, altura * 0.9)
                    );
                }
            }
        }
    }

    /// Menor distância de `(px, py)` até a curva da meia elipse (o topo reto é a borda da tela e
    /// não conta).
    fn distancia_a_curva(w: f64, h: f64, px: f64, py: f64) -> f64 {
        let a = w / 2.0;
        (0..=4000)
            .map(|k| {
                let t = std::f64::consts::PI * k as f64 / 4000.0;
                let (cx, cy) = (a - a * t.cos(), h * t.sin());
                ((cx - px).powi(2) + (cy - py).powi(2)).sqrt()
            })
            .fold(f64::MAX, f64::min)
    }

    /// Controles (px lógicos, medidas da spec rev. 2 e do `pilula.html`) ficam dentro da curva com
    /// folga de pelo menos 6 px nos modos aberto e gravando. Cada controle é uma caixa
    /// `(largura, altura, raio)` na linha centrada em X, com `gap` de 8 px; o contorno real
    /// (pílula, círculo ou retângulo) é amostrado e medido contra a curva.
    #[test]
    fn controles_cabem_dentro_da_curva() {
        let pilula_ = |w: f64, h: f64| (w, h, h / 2.0);
        let caixa = |w: f64, h: f64| (w, h, 0.0);
        // (rótulo, W, H, centro da linha, gap, controles)
        type Caso = (&'static str, f64, f64, f64, f64, Vec<(f64, f64, f64)>);
        let casos: [Caso; 5] = [
            (
                "aberta, ociosa",
                248.0,
                54.0,
                22.0,
                8.0,
                vec![pilula_(84.0, 32.0), pilula_(72.0, 24.0)],
            ),
            (
                "gravando, aberta",
                248.0,
                54.0,
                22.0,
                8.0,
                vec![caixa(51.0, 22.0), caixa(30.0, 18.0), pilula_(32.0, 32.0)],
            ),
            (
                "mãos livres",
                284.0,
                56.0,
                23.0,
                8.0,
                vec![
                    pilula_(32.0, 32.0),
                    caixa(51.0, 22.0),
                    caixa(30.0, 18.0),
                    pilula_(32.0, 32.0),
                ],
            ),
            (
                "gravando, compacta",
                116.0,
                40.0,
                17.0,
                0.0,
                vec![caixa(31.0, 18.0)],
            ),
            (
                "baixando, compacta",
                132.0,
                42.0,
                18.0,
                6.0,
                vec![caixa(16.0, 16.0), caixa(28.0, 16.0)],
            ),
        ];
        for (rotulo, w, h, centro, gap, controles) in casos {
            let total: f64 =
                controles.iter().map(|c| c.0).sum::<f64>() + gap * (controles.len() - 1) as f64;
            let mut x = (w - total) / 2.0;
            let mut menor = f64::MAX;
            for (cw, ch, r) in controles {
                let (x0, y0) = (x, centro - ch / 2.0);
                // Contorno: lados retos + cantos arredondados de raio r (r = 0: retângulo).
                for k in 0..360 {
                    let t = (k as f64).to_radians();
                    let (dx, dy) = (t.cos(), t.sin());
                    let cx = if dx < 0.0 { x0 + r } else { x0 + cw - r };
                    let cy = if dy < 0.0 { y0 + r } else { y0 + ch - r };
                    let (px, py) = if r > 0.0 {
                        (cx + r * dx, cy + r * dy)
                    } else {
                        (
                            if dx < 0.0 { x0 } else { x0 + cw },
                            if dy < 0.0 { y0 } else { y0 + ch },
                        )
                    };
                    assert!(
                        dentro_da_meia_elipse(w, h, px, py),
                        "{rotulo}: ({px},{py}) fora"
                    );
                    menor = menor.min(distancia_a_curva(w, h, px, py));
                }
                x += cw + gap;
            }
            assert!(menor >= 6.0, "{rotulo}: folga de {menor:.1} px até a curva");
        }
    }

    fn tela_runner(w: i32, h: i32, escala: f64) -> RetanguloMonitor {
        RetanguloMonitor {
            x: 0,
            y: 0,
            w,
            h,
            work_x: 0,
            work_y: 0,
            work_w: w,
            work_h: h - 40,
            escala,
            principal: true,
        }
    }

    fn ret(p: PosicaoIlha) -> Retangulo {
        (p.x, p.y, p.w, p.h)
    }

    /// Defeito 1 do runner: 1024x768 -> 1920x1080 a 100% deixava a ilha em x=466.
    #[test]
    fn troca_de_resolucao_reaplica_no_centro_novo() {
        let antes = ret(escolher_topo(&[tela_runner(1024, 768, 1.0)], 92.0, 7.0).unwrap());
        assert_eq!(antes, (466, 0, 92, 7));
        let depois = ret(escolher_topo(&[tela_runner(1920, 1080, 1.0)], 92.0, 7.0).unwrap());
        assert_eq!(depois, (914, 0, 92, 7));
        assert_eq!(
            decidir_conserto(96, 96, Some(antes), Some(antes), Some(depois)),
            Conserto::Reaplicar
        );
    }

    /// Defeito 2: escala trocada com o app aberto e a janela presa em 96 DPI.
    #[test]
    fn janela_presa_em_dpi_antigo_e_recriada() {
        for (dpi, escala, esperado) in [
            (120, 1.25, (903, 0, 115, 9)),
            (144, 1.5, (891, 0, 138, 11)),
            (168, 1.75, (880, 0, 161, 12)),
        ] {
            let calc = ret(escolher_topo(&[tela_runner(1920, 1080, escala)], 92.0, 7.0).unwrap());
            assert_eq!(calc, esperado, "escala {escala}");
            let antigo = Some((466, 0, 92, 7));
            assert_eq!(
                decidir_conserto(96, dpi, antigo, antigo, Some(calc)),
                Conserto::Recriar
            );
            // Depois de recriada (DPI certo), só falta posicionar.
            assert_eq!(
                decidir_conserto(dpi, dpi, None, Some((0, 0, 24, 24)), Some(calc)),
                Conserto::Reaplicar
            );
        }
    }

    /// Defeito 3: abertura em 175% com a ilha 24x24 em (0,0) depois do SetWindowPos do tao.
    #[test]
    fn deriva_depois_do_tao_e_reaplicada() {
        let calc = ret(escolher_topo(&[tela_runner(1920, 1080, 1.75)], 92.0, 7.0).unwrap());
        assert_eq!(calc, (880, 0, 161, 12));
        assert_eq!(
            decidir_conserto(168, 168, Some(calc), Some((0, 0, 24, 24)), Some(calc)),
            Conserto::Reaplicar
        );
        assert_eq!(
            decidir_conserto(168, 168, Some(calc), Some(calc), Some(calc)),
            Conserto::Nada
        );
    }

    #[test]
    fn leitura_de_dpi_indisponivel_nao_recria() {
        let r = Some((466, 0, 92, 7));
        assert_eq!(decidir_conserto(0, 168, r, r, r), Conserto::Nada);
        assert_eq!(decidir_conserto(96, 0, r, r, r), Conserto::Nada);
        assert_eq!(decidir_conserto(96, 96, r, r, None), Conserto::Nada);
    }

    #[test]
    fn limite_impede_laco_e_libera_depois_da_janela() {
        let mut l = Limite::novo(Duration::from_secs(3), 2);
        let t0 = Instant::now();
        assert!(l.permitir(t0));
        assert!(l.permitir(t0 + Duration::from_millis(10)));
        assert!(!l.permitir(t0 + Duration::from_millis(20)));
        assert!(l.permitir(t0 + Duration::from_secs(3)));
    }

    #[test]
    fn conferencia_roda_depois_do_tao_e_de_novo_mais_tarde() {
        assert!(ATRASOS_CONFERENCIA[0] >= Duration::from_millis(50));
        assert!(ATRASOS_CONFERENCIA[1] > ATRASOS_CONFERENCIA[0]);
    }

    /// Os gatilhos existem no código: escala só agenda (não posiciona dentro do WM_DPICHANGED do
    /// tao), e a camada nativa ouve resolução, configuração, deriva e DPI.
    #[test]
    fn gatilhos_de_tela_estao_ligados() {
        let pilula = include_str!("pilula.rs");
        let producao = pilula
            .split("#[cfg(test)]\nmod testes")
            .next()
            .unwrap_or(pilula);
        let trecho = producao
            .split("WindowEvent::ScaleFactorChanged")
            .nth(1)
            .and_then(|t| t.split("Some(j)").next())
            .unwrap_or("");
        assert!(trecho.contains("reconferir(\"escala\")"));
        assert!(!trecho.contains("aplicar_janela"));
        assert!(producao.contains("reconferir(\"abertura\")"));
        let nativo = include_str!("nativo/win_janela.rs");
        for exigido in [
            "WM_DISPLAYCHANGE",
            "WM_SETTINGCHANGE",
            "WM_DPICHANGED",
            "EVENT_OBJECT_LOCATIONCHANGE",
            "GetDpiForWindow",
            "GetDpiForMonitor",
            "GetCurrentProcessId",
        ] {
            assert!(nativo.contains(exigido), "falta {exigido}");
        }
    }

    #[test]
    fn protocolo_de_modo_aceita_geometria_exata_e_recusa_absurdos() {
        assert_eq!(interpretar_modo("aberta"), Some((ModoJanela::Aberta, None)));
        assert_eq!(
            interpretar_modo("aberta:248:48"),
            Some((ModoJanela::Aberta, Some((248.0, 48.0))))
        );
        assert_eq!(
            interpretar_modo("repouso:104:8"),
            Some((ModoJanela::Repouso, Some((104.0, 8.0))))
        );
        for ruim in [
            "lateral",
            "aberta:248",
            "aberta:248:48:24",
            "aberta:x:48",
            "aberta:4000:48",
            "aberta:248:900",
            "aberta:NaN:48",
            "aberta:inf:48",
        ] {
            assert_eq!(interpretar_modo(ruim), None, "{ruim}");
        }
    }

    #[test]
    fn troca_de_modo_pelo_rust_volta_ao_tamanho_padrao() {
        let mut i = interno_teste();
        i.geometria = (104.0, 8.0); // aceno informado pela página
        let _ = atualizar_modo_e_vigia(&mut i, ModoJanela::Repouso);
        assert_eq!(
            i.geometria,
            (104.0, 8.0),
            "mesmo modo preserva a geometria da página"
        );
        let _ = atualizar_modo_e_vigia(&mut i, ModoJanela::Compacta);
        assert_eq!(i.geometria, ModoJanela::Compacta.dimensoes_padrao());
    }

    #[test]
    fn janela_nativa_recorta_e_nunca_ativa() {
        let codigo = include_str!("nativo/win_janela.rs");
        let producao = codigo.split("#[cfg(test)]").next().unwrap_or(codigo);
        for exigido in [
            "WS_EX_NOACTIVATE",
            "WS_EX_TOOLWINDOW",
            "WS_EX_TOPMOST",
            "SWP_NOACTIVATE",
            "CreateEllipticRgn",
            "SetWindowRgn",
        ] {
            assert!(
                producao.contains(exigido),
                "falta {exigido} em win_janela.rs"
            );
        }
        for proibido in [
            "ShowWindow(",
            "SetForegroundWindow",
            "SetActiveWindow",
            "SetFocus(",
        ] {
            assert!(
                !producao.contains(proibido),
                "win_janela.rs não pode usar {proibido}"
            );
        }
        let pilula = include_str!("pilula.rs");
        let producao = pilula
            .split("#[cfg(test)]\nmod testes")
            .next()
            .unwrap_or(pilula);
        assert!(
            producao.contains("pub fn aplicar_janela"),
            "corte do código de produção errado"
        );
        assert!(producao.contains("recortar_elipse"));
        assert!(producao.contains("posicionar_sem_ativar"));
    }

    #[test]
    fn pilula_nunca_chama_show_do_tauri() {
        let codigo = include_str!("pilula.rs");
        let producao = codigo
            .split("#[cfg(test)]\nmod testes")
            .next()
            .unwrap_or(codigo);
        assert!(
            !producao.contains(".show()"),
            "pilula.rs não pode chamar WebviewWindow::show(), que pode ativar a janela"
        );
    }

    fn interno_teste() -> Interno {
        Interno {
            fase: Fase::Ociosa,
            modo: ModoJanela::Repouso,
            sempre_visivel: true,
            tela_cheia: false,
            baixando_modelo: None,
            progresso_ativo: false,
            aviso_ate: None,
            geometria: ModoJanela::Repouso.dimensoes_padrao(),
            ultimo_nivel: Instant::now(),
            atalho: "Ctrl dir.".into(),
            canal: None,
            vigia_geracao_ativa: Arc::new(AtomicU64::new(0)),
            vigia_proxima_geracao: 0,
            limite_reaplicar: Limite::novo(Duration::from_secs(3), 8),
            limite_recriar: Limite::novo(Duration::from_secs(20), 2),
        }
    }

    #[test]
    fn sair_de_aberta_desliga_vigia_de_cursor_em_repouso() {
        let mut i = interno_teste();
        // Hover abre -> vigia liga com geração 1.
        let (ativa, gen1) = atualizar_modo_e_vigia(&mut i, ModoJanela::Aberta).unwrap();
        assert!(vigia_ainda_valido(&ativa, gen1));
        // Atalho inicia ditado (Compacta) -> vigia desliga imediatamente (I-3).
        assert!(atualizar_modo_e_vigia(&mut i, ModoJanela::Compacta).is_none());
        assert!(!vigia_ainda_valido(&ativa, gen1));
        // Volta a repouso -> vigia segue desligado.
        assert!(atualizar_modo_e_vigia(&mut i, ModoJanela::Repouso).is_none());
        assert!(!vigia_ainda_valido(&ativa, gen1));
    }

    #[test]
    fn abrir_e_progresso_repetido_preserva_janela_aberta_e_sai_ao_fechar() {
        let mut i = interno_teste();
        // Chega progresso com aba em repouso -> passa a Compacta.
        i.progresso_ativo = true;
        let modo = modo_para_atividade_ociosa(&i);
        assert_eq!(modo, ModoJanela::Compacta);
        let _ = atualizar_modo_e_vigia(&mut i, modo);

        // Ponteiro abre a aba -> Aberta (vigia ativo).
        let (ativa, gen1) = atualizar_modo_e_vigia(&mut i, ModoJanela::Aberta).unwrap();
        assert_eq!(i.modo, ModoJanela::Aberta);
        assert!(vigia_ainda_valido(&ativa, gen1));

        // Novos eventos de progresso ou modelo repetidos não podem encolher a aba aberta.
        for feito in [10u64, 40, 80] {
            i.baixando_modelo = Some((feito, 100));
            let m = modo_para_atividade_ociosa(&i);
            assert_eq!(m, ModoJanela::Aberta);
            assert!(atualizar_modo_e_vigia(&mut i, m).is_none());
            assert!(vigia_ainda_valido(&ativa, gen1));
        }

        // Quando o ponteiro sai (pilula.js envia modo:compacta), volta a Compacta e desliga vigia.
        assert!(atualizar_modo_e_vigia(&mut i, ModoJanela::Compacta).is_none());
        assert_eq!(i.modo, ModoJanela::Compacta);
        assert!(!vigia_ainda_valido(&ativa, gen1));

        // Ao encerrar o progresso/download fora do hover, volta a Repouso.
        i.baixando_modelo = None;
        i.progresso_ativo = false;
        let m = modo_para_atividade_ociosa(&i);
        assert_eq!(m, ModoJanela::Repouso);
        let _ = atualizar_modo_e_vigia(&mut i, m);
        assert_eq!(i.modo, ModoJanela::Repouso);
    }

    #[test]
    fn reabrir_rapido_invalida_vigia_anterior_antes_do_proximo_ciclo() {
        let mut i = interno_teste();
        // 1. Abrir -> gera vigia da geração 1.
        let (ativa, gen1) = atualizar_modo_e_vigia(&mut i, ModoJanela::Aberta).unwrap();
        assert!(vigia_ainda_valido(&ativa, gen1));
        // 2. Fechar -> geração ativa vai a 0.
        assert!(atualizar_modo_e_vigia(&mut i, ModoJanela::Repouso).is_none());
        assert!(!vigia_ainda_valido(&ativa, gen1));
        // 3. Reabrir rápido (antes do próximo ciclo de 100 ms do vigia 1) -> geração 2.
        let (ativa2, gen2) = atualizar_modo_e_vigia(&mut i, ModoJanela::Aberta).unwrap();
        assert_ne!(gen1, gen2);
        // O vigia anterior (gen1) continua inválido e sai no próximo ciclo sem emitir eventos.
        assert!(!vigia_ainda_valido(&ativa, gen1));
        // Apenas o vigia novo (gen2) segue válido.
        assert!(vigia_ainda_valido(&ativa2, gen2));
    }

    #[test]
    fn timer_de_aviso_nao_encolhe_sob_hover_com_canal_conectado() {
        // Canal conectado + ponteiro dentro da aba -> não encolhe pelo Rust (I-4).
        assert!(!deve_encolher_aviso_no_timer(true, false));
        // Canal conectado + ponteiro fora -> pode encolher.
        assert!(deve_encolher_aviso_no_timer(true, true));
        // Sem canal conectado -> timer de segurança sempre encolhe.
        assert!(deve_encolher_aviso_no_timer(false, false));
    }
}
