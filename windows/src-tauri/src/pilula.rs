//! Ilha no centro da borda superior do Windows (v1.0.2, SPEC-dynamic-island.md, com o ajuste de
//! aceite do Founder: base em curva contínua). Segunda janela Tauri transparente, sempre no topo,
//! fora da barra de tarefas e sem nunca roubar o foco (`WS_EX_NOACTIVATE`, posicionamento e
//! exibição só por `SetWindowPos` com `SWP_NOACTIVATE`, nunca `WebviewWindow::show()`).
//!
//! Forma (SPEC-windows-identidade.md §2): barrinha em repouso e cápsula escura solta 6 DIP do
//! topo, raio = altura / 2. A janela fica presa em `y = work_y`; a região de clique é a cápsula
//! 1 px físico maior (`CreateRoundRectRgn`), e o vão de cima e as quinas atravessam o clique.
//! Histórico: a v1.0.2 inicial usava meia elipse com o topo reto rente à borda (centro em `(w/2, 0)`,
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
const JANELA_W: (f64, f64) = (40.0, 380.0);
const JANELA_H: (f64, f64) = (8.0, 64.0);

/// Geometria pedida pela página (px lógicos): tamanho da janela (sempre presa em `y = work_y`) e
/// a cápsula dentro dela (centrada em X, topo em `cap_y`). Com `recorte_capsula`, a região de
/// clique é a cápsula 1 px físico maior; sem, a janela inteira (barrinha e transições).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Geometria {
    pub w: f64,
    pub h: f64,
    pub cap_w: f64,
    pub cap_h: f64,
    pub cap_y: f64,
    pub recorte_capsula: bool,
}

impl ModoJanela {
    /// Tamanhos da §2.3 usados enquanto a página não informa os exatos.
    pub fn geometria_padrao(self) -> Geometria {
        let (cap_w, cap_h, cap_y, w, h, recorte_capsula) = match self {
            ModoJanela::Repouso => (40.0, 5.0, 3.0, 72.0, 14.0, false),
            ModoJanela::Compacta => (112.0, 30.0, 6.0, 116.0, 38.0, true),
            ModoJanela::Aberta => (184.0, 36.0, 6.0, 188.0, 44.0, true),
        };
        Geometria {
            w,
            h,
            cap_w,
            cap_h,
            cap_y,
            recorte_capsula,
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

/// Interpreta o pedido da página: `aberta` ou
/// `aberta:{janela w}:{janela h}:{cápsula w}:{cápsula h}:{cápsula y}:{c|r}` (px lógicos).
/// Medidas fora dos limites, ou cápsula fora da janela, são recusadas.
pub fn interpretar_modo(s: &str) -> Option<(ModoJanela, Option<Geometria>)> {
    let partes: Vec<&str> = s.split(':').collect();
    let modo = ModoJanela::de_nome(partes.first()?)?;
    if partes.len() == 1 {
        return Some((modo, None));
    }
    if partes.len() != 7 {
        return None;
    }
    let n: Vec<f64> = partes[1..6]
        .iter()
        .map(|p| p.parse::<f64>().ok().filter(|v| v.is_finite()))
        .collect::<Option<_>>()?;
    let recorte_capsula = match partes[6] {
        "c" => true,
        "r" => false,
        _ => return None,
    };
    let g = Geometria {
        w: n[0],
        h: n[1],
        cap_w: n[2],
        cap_h: n[3],
        cap_y: n[4],
        recorte_capsula,
    };
    let valido = (JANELA_W.0..=JANELA_W.1).contains(&g.w)
        && (JANELA_H.0..=JANELA_H.1).contains(&g.h)
        && g.cap_w > 0.0
        && g.cap_h > 0.0
        && g.cap_y >= 0.0
        && g.cap_w <= g.w
        && g.cap_y + g.cap_h <= g.h;
    valido.then_some((modo, Some(g)))
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
    let w = (w_log * m.escala).ceil() as i32;
    let h = (h_log * m.escala).ceil() as i32;
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

/// Região de clique da cápsula em pixels físicos relativos à janela de largura `janela_w`:
/// `(esquerda, topo, direita, base, diâmetro do canto)` para `CreateRoundRectRgn`, 1 px físico
/// maior que a cápsula em toda a volta (§2.7: a região nunca pode cortar o desenho antisserrilhado).
#[cfg_attr(not(windows), allow(dead_code))]
pub fn regiao_capsula(janela_w: i32, g: &Geometria, escala: f64) -> (i32, i32, i32, i32, i32) {
    let cap_w = (g.cap_w * escala).round() as i32;
    let cap_h = (g.cap_h * escala).round() as i32;
    let cap_t = (g.cap_y * escala).round() as i32;
    let cap_l = (janela_w - cap_w) / 2;
    (
        cap_l - 1,
        cap_t - 1,
        cap_l + cap_w + 1,
        cap_t + cap_h + 1,
        cap_h + 2,
    )
}

/// Teste de ponto da região (`CreateRoundRectRgn` exclui a borda direita e a de baixo).
#[cfg(test)]
pub fn dentro_da_regiao(r: (i32, i32, i32, i32, i32), px: f64, py: f64) -> bool {
    let (l, t, rr, b, d) = r;
    let (l, t, rr, b) = (l as f64, t as f64, rr as f64, b as f64);
    if px < l || px >= rr || py < t || py >= b {
        return false;
    }
    let raio = d as f64 / 2.0;
    let cx = px.clamp(l + raio, rr - raio);
    let cy = py.clamp(t + raio, b - raio);
    (px - cx).powi(2) + (py - cy).powi(2) <= raio * raio
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
    regiao_ok: bool,
) -> Conserto {
    if dpi_janela != 0 && dpi_monitor != 0 && dpi_janela != dpi_monitor {
        return Conserto::Recriar;
    }
    // Achado do Par: depois de recriar, a região às vezes volta a ser retângulo (tipo 2) sem mudar
    // o retângulo da janela; os cantos passariam a engolir o clique.
    if calculado.is_some() && esperado.is_some() && !regiao_ok {
        return Conserto::Reaplicar;
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

    /// Quanto falta para liberar a próxima ação (zero se já está livre).
    pub fn espera(&self, agora: Instant) -> Duration {
        if self.marcas.len() < self.max {
            return Duration::ZERO;
        }
        self.marcas
            .front()
            .map(|t| (*t + self.janela).saturating_duration_since(agora))
            .unwrap_or(Duration::ZERO)
    }
}

/// Atrasos da conferência depois de um aviso de tela: o primeiro roda depois do `SetWindowPos`
/// que o tao faz ao fim do `WM_DPICHANGED`; o segundo pega a área de trabalho atualizada tarde.
pub const ATRASOS_CONFERENCIA: [Duration; 2] =
    [Duration::from_millis(120), Duration::from_millis(700)];

/// Depois de abrir ou recriar a janela, conferências extras: o WebView2 termina de montar mais
/// tarde, e é nessa janela de tempo que a região já foi vista voltando a retângulo.
pub const ATRASOS_DEPOIS_DE_CRIAR: [Duration; 4] = [
    Duration::from_millis(120),
    Duration::from_millis(700),
    Duration::from_millis(2000),
    Duration::from_millis(5000),
];

pub fn atrasos_para(motivo: &str) -> &'static [Duration] {
    match motivo {
        "abertura" | "recriada" => &ATRASOS_DEPOIS_DE_CRIAR,
        _ => &ATRASOS_CONFERENCIA,
    }
}

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
    saida_ate: Option<Instant>,
    /// Tamanho lógico atual da ilha (largura exata do estado informada pela página).
    geometria: Geometria,
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
        i.geometria = modo.geometria_padrao();
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
    retentativa_agendada: AtomicBool,
    geracao_conferencia: AtomicU64,
}

/// Cria a janela da ilha: transparente, sem foco, fora da barra de tarefas, escondida até o
/// primeiro `SetWindowPos`. Também usada para recriá-la quando o DPI dela fica preso.
fn construir_janela(app: &AppHandle, eu: Weak<Pilula>) -> Option<WebviewWindow> {
    let g = ModoJanela::Repouso.geometria_padrao();
    let (lw, lh) = (g.w, g.h);
    let janela = WebviewWindowBuilder::new(app, ROTULO, WebviewUrl::App("pilula.html".into()))
        .title("Dita.se")
        .inner_size(lw, lh)
        .resizable(false)
        .decorations(false)
        .transparent(true)
        // Fundo do WebView2 com alfa 0 (§2.7): nada cinza ou branco fora da cápsula.
        .background_color(tauri::window::Color(0, 0, 0, 0))
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
                saida_ate: None,
                geometria: ModoJanela::Repouso.geometria_padrao(),
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
            retentativa_agendada: AtomicBool::new(false),
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
                let mut decorrido = Duration::ZERO;
                for (n, atraso) in atrasos_para(motivo).iter().enumerate() {
                    std::thread::sleep(atraso.saturating_sub(decorrido));
                    decorrido = *atraso;
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

    /// Uma única nova conferência daqui a `atraso` (pedidos repetidos enquanto uma está agendada
    /// são ignorados).
    #[cfg_attr(not(windows), allow(dead_code))]
    fn reconferir_depois(&self, atraso: Duration) {
        if self.retentativa_agendada.swap(true, Ordering::AcqRel) {
            return;
        }
        let Some(eu) = self.eu.get().cloned() else {
            self.retentativa_agendada.store(false, Ordering::Release);
            return;
        };
        let _ = std::thread::Builder::new()
            .name("ditase.ilha.retentativa".into())
            .spawn(move || {
                std::thread::sleep(atraso);
                if let Some(p) = eu.upgrade() {
                    p.retentativa_agendada.store(false, Ordering::Release);
                    p.conferir("retentativa");
                }
            });
    }

    /// Compara a ilha com a tela de agora e conserta (reaplica ou recria), com limite contra laço.
    #[cfg(windows)]
    fn conferir(&self, motivo: &'static str) {
        if self.recriando.load(Ordering::Acquire) {
            return;
        }
        let Some(j) = self.janela() else {
            return;
        };
        let (visivel, g) = {
            let i = self.interno.lock().unwrap();
            (Self::deve_estar_visivel(&i), i.geometria)
        };
        let (w_log, h_log) = (g.w, g.h);
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
        let (tipo_regiao, canto_fora) = crate::nativo::janela::estado_da_regiao(hwnd);
        // Com cápsula, a região tem de ser complexa e deixar o canto de baixo fora; na barrinha e
        // na transição, a janela inteira pega o clique e não há o que conferir.
        let regiao_ok = !g.recorte_capsula || crate::nativo::janela::regiao_recortada(hwnd);
        let conserto = decidir_conserto(
            dpi_janela,
            dpi_monitor,
            esperado,
            real,
            calculado,
            regiao_ok,
        );
        if conserto == Conserto::Nada && motivo != "abertura" {
            return;
        }
        let (permitido, espera) = {
            let mut i = self.interno.lock().unwrap();
            let agora = Instant::now();
            let limite = match conserto {
                Conserto::Nada => None,
                Conserto::Reaplicar => Some(&mut i.limite_reaplicar),
                Conserto::Recriar => Some(&mut i.limite_recriar),
            };
            match limite {
                None => (true, Duration::ZERO),
                Some(l) => {
                    if l.permitir(agora) {
                        (true, Duration::ZERO)
                    } else {
                        (false, l.espera(agora))
                    }
                }
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
             real {real:?} calculado {calculado:?}; regiao tipo {tipo_regiao} canto fora \
             {canto_fora}; telas [{}] -> {conserto:?}{}",
            telas.join("; "),
            if permitido {
                ""
            } else {
                " (limite atingido, ignorado)"
            }
        ));
        if !permitido {
            // Nova tentativa quando o limite libera, para a ilha não ficar no DPI ou na posição
            // antigos até o próximo evento de tela.
            self.reconferir_depois(espera + Duration::from_millis(50));
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
            || i.saida_ate.is_some_and(|ate| Instant::now() < ate)
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
        let (visivel, g) = {
            let i = self.interno.lock().unwrap();
            (Self::deve_estar_visivel(&i), i.geometria)
        };
        let (w_log, h_log) = (g.w, g.h);
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
            let escala = pos.w as f64 / g.w;
            let regiao = g.recorte_capsula.then(|| regiao_capsula(pos.w, &g, escala));
            crate::nativo::janela::recortar(hwnd, regiao);
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
                        // Zona de manter aberta (§2.7): 16 DIP para os lados e para baixo, e até a
                        // borda de cima da tela.
                        let hwnd = h.0 as isize;
                        let margem = (16.0 * crate::nativo::janela::dpi_da_janela(hwnd) as f64
                            / 96.0)
                            .round() as i32;
                        if crate::nativo::janela::cursor_fora_da_zona(hwnd, margem, margem, true) {
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

    /// Pedido da página (`modo:{nome}[:…7 campos]`): troca o modo e, se vier, usa a geometria exata
    /// do estado. A página só pede para encolher depois que a animação de saída terminou.
    pub fn definir_modo(&self, modo: ModoJanela, geometria: Option<Geometria>) {
        let iniciar = {
            let mut i = self.interno.lock().unwrap();
            if modo != ModoJanela::Aberta {
                // Se o JS comandou o fechamento (ex.: fim de aviso após sair o hover), limpa aviso_ate.
                i.aviso_ate = None;
            }
            let antes = (i.modo, i.geometria);
            let flag = atualizar_modo_e_vigia(&mut i, modo);
            let recolheu = modo == ModoJanela::Repouso
                && geometria.is_some_and(|g| g.w == 72.0 && g.h == 14.0);
            let fim_da_saida = recolheu && i.saida_ate.take().is_some();
            if let Some(g) = geometria {
                i.geometria = g;
            }
            if antes == (i.modo, i.geometria) && flag.is_none() && !fim_da_saida {
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
        let (iniciar, saida) = {
            let mut i = self.interno.lock().unwrap();
            let saindo = i.fase == Fase::Processando && fase == Fase::Ociosa && i.canal.is_some();
            i.fase = fase;
            if fase != Fase::Ociosa {
                i.aviso_ate = None;
                i.saida_ate = None;
            }
            if saindo {
                // A página ainda precisa da cápsula para apagar o segmento antes de recolher.
                let ate = Instant::now() + Duration::from_millis(450);
                i.saida_ate = Some(ate);
                (None, Some(ate))
            } else if i.saida_ate.is_some() {
                (None, None)
            } else {
                let modo = modo_da_fase(&i);
                (atualizar_modo_e_vigia(&mut i, modo), None)
            }
        };
        self.aplicar_janela();
        self.iniciar_vigia_se_preciso(iniciar);
        self.enviar(msg_fase(fase));

        if let Some(ate) = saida {
            let app = self.app.clone();
            std::thread::spawn(move || {
                std::thread::sleep(ate.saturating_duration_since(Instant::now()));
                let app2 = app.clone();
                let _ = app.run_on_main_thread(move || {
                    let Some(casca) = app2.try_state::<Arc<crate::Casca>>() else {
                        return;
                    };
                    let p = &casca.pilula;
                    let mut i = p.interno.lock().unwrap();
                    if i.saida_ate != Some(ate) {
                        return;
                    }
                    i.saida_ate = None;
                    if i.canal.is_none() {
                        let modo = modo_da_fase(&i);
                        atualizar_modo_e_vigia(&mut i, modo);
                    }
                    drop(i);
                    p.aplicar_janela();
                });
            });
        }
    }

    /// Mensagem curta com nível (`info` ou `erro`); a aba volta sozinha a ociosa depois de `segundos`.
    pub fn avisar(&self, texto: &str, segundos: f64, nivel: NivelAviso) {
        let duracao = match nivel {
            NivelAviso::Erro => segundos.max(3.5),
            NivelAviso::Info => segundos,
        };
        let iniciar = {
            let mut i = self.interno.lock().unwrap();
            let manter_forma = i.fase == Fase::Processando || i.saida_ate.is_some();
            let geometria = i.geometria;
            i.fase = Fase::Ociosa;
            i.saida_ate = None;
            i.aviso_ate = Some(Instant::now() + Duration::from_secs_f64(duracao + 0.4));
            let vigia = atualizar_modo_e_vigia(&mut i, ModoJanela::Aberta);
            if manter_forma {
                i.geometria = geometria;
            }
            vigia
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
                        .is_none_or(|h| {
                            crate::nativo::janela::cursor_fora_da_zona(h.0 as isize, 0, 0, false)
                        })
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
                assert_eq!(pos.w, (w_log * escala).ceil() as i32);
                assert_eq!(pos.h, (h_log * escala).ceil() as i32);
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
        let antes = ret(escolher_topo(&[tela_runner(1024, 768, 1.0)], 72.0, 14.0).unwrap());
        assert_eq!(antes, (476, 0, 72, 14));
        let depois = ret(escolher_topo(&[tela_runner(1920, 1080, 1.0)], 72.0, 14.0).unwrap());
        assert_eq!(depois, (924, 0, 72, 14));
        assert_eq!(
            decidir_conserto(96, 96, Some(antes), Some(antes), Some(depois), true),
            Conserto::Reaplicar
        );
    }

    /// Defeito 2: escala trocada com o app aberto e a janela presa em 96 DPI.
    #[test]
    fn janela_presa_em_dpi_antigo_e_recriada() {
        for (dpi, escala, esperado) in [
            (120, 1.25, (915, 0, 90, 18)),
            (144, 1.5, (906, 0, 108, 21)),
            (168, 1.75, (897, 0, 126, 25)),
        ] {
            let calc = ret(escolher_topo(&[tela_runner(1920, 1080, escala)], 72.0, 14.0).unwrap());
            assert_eq!(calc, esperado, "escala {escala}");
            let antigo = Some((476, 0, 72, 14));
            assert_eq!(
                decidir_conserto(96, dpi, antigo, antigo, Some(calc), true),
                Conserto::Recriar
            );
            // Depois de recriada (DPI certo), só falta posicionar.
            assert_eq!(
                decidir_conserto(dpi, dpi, None, Some((0, 0, 24, 24)), Some(calc), true),
                Conserto::Reaplicar
            );
        }
    }

    /// Defeito 3: abertura em 175% com a ilha 24x24 em (0,0) depois do SetWindowPos do tao.
    #[test]
    fn deriva_depois_do_tao_e_reaplicada() {
        let calc = ret(escolher_topo(&[tela_runner(1920, 1080, 1.75)], 72.0, 14.0).unwrap());
        assert_eq!(calc, (897, 0, 126, 25));
        assert_eq!(
            decidir_conserto(168, 168, Some(calc), Some((0, 0, 24, 24)), Some(calc), true),
            Conserto::Reaplicar
        );
        assert_eq!(
            decidir_conserto(168, 168, Some(calc), Some(calc), Some(calc), true),
            Conserto::Nada
        );
    }

    /// Achado do Par (run 44888186295): depois de recriar, retângulo certo mas região tipo 2.
    #[test]
    fn regiao_retangular_com_retangulo_certo_e_reaplicada() {
        let r = Some((906, 0, 108, 21));
        assert_eq!(
            decidir_conserto(144, 144, r, r, r, false),
            Conserto::Reaplicar
        );
        assert_eq!(decidir_conserto(144, 144, r, r, r, true), Conserto::Nada);
        // DPI preso continua pedindo recriação antes de tudo.
        assert_eq!(
            decidir_conserto(144, 168, r, r, r, false),
            Conserto::Recriar
        );
        // Escondida (sem retângulo pedido): nada a recortar.
        assert_eq!(
            decidir_conserto(144, 144, None, None, None, false),
            Conserto::Nada
        );
    }

    #[test]
    fn depois_de_criar_confere_tambem_mais_tarde() {
        assert_eq!(
            atrasos_para("recriada").last(),
            Some(&Duration::from_millis(5000))
        );
        assert_eq!(atrasos_para("abertura").len(), 4);
        assert_eq!(atrasos_para("configuracao"), &ATRASOS_CONFERENCIA[..]);
        let nativo = include_str!("nativo/win_janela.rs");
        assert!(nativo.contains("GetWindowRgn") && nativo.contains("PtInRegion"));
    }

    #[test]
    fn leitura_de_dpi_indisponivel_nao_recria() {
        let r = Some((476, 0, 72, 14));
        assert_eq!(decidir_conserto(0, 168, r, r, r, true), Conserto::Nada);
        assert_eq!(decidir_conserto(96, 0, r, r, r, true), Conserto::Nada);
        assert_eq!(decidir_conserto(96, 96, r, r, None, true), Conserto::Nada);
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

    /// Achado do Par: a recriação barrada pelo limite precisa de nova tentativa quando ele libera.
    #[test]
    fn limite_informa_quando_libera_para_a_nova_tentativa() {
        let mut l = Limite::novo(Duration::from_secs(20), 2);
        let t0 = Instant::now();
        assert_eq!(l.espera(t0), Duration::ZERO);
        assert!(l.permitir(t0));
        assert!(l.permitir(t0 + Duration::from_millis(6500)));
        let t3 = t0 + Duration::from_millis(13000);
        assert!(!l.permitir(t3));
        assert_eq!(l.espera(t3), Duration::from_millis(7000));
        assert!(l.permitir(t3 + l.espera(t3)));
        let producao = include_str!("pilula.rs");
        let producao = producao
            .split("#[cfg(test)]\nmod testes")
            .next()
            .unwrap_or(producao);
        assert!(producao.contains("self.reconferir_depois(espera"));
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

    fn geo(w: f64, h: f64, cap_w: f64, cap_h: f64, cap_y: f64, c: bool) -> Geometria {
        Geometria {
            w,
            h,
            cap_w,
            cap_h,
            cap_y,
            recorte_capsula: c,
        }
    }

    /// Região da cápsula (§2.7): 1 px físico maior que o desenho, quinas e vão de cima de fora.
    #[test]
    fn regiao_e_capsula_um_pixel_maior_com_quinas_e_vao_de_fora() {
        for escala in [1.0_f64, 1.25, 1.5, 1.75, 2.0] {
            for (cap_w, cap_h) in [(184.0, 36.0), (112.0, 30.0), (252.0, 36.0), (360.0, 52.0)] {
                let g = geo(cap_w + 4.0, 6.0 + cap_h + 2.0, cap_w, cap_h, 6.0, true);
                let jw = (g.w * escala).round() as i32;
                let jh = (g.h * escala).round() as i32;
                let r = regiao_capsula(jw, &g, escala);
                let (l, t, rr, b, d) = r;
                let cw = (cap_w * escala).round() as i32;
                let ch = (cap_h * escala).round() as i32;
                let ct = (6.0 * escala).round() as i32;
                assert_eq!((rr - l, b - t, d), (cw + 2, ch + 2, ch + 2));
                assert_eq!(t, ct - 1);
                assert!(l >= 0 && rr <= jw && b <= jh, "região dentro da janela");
                // Toda a cápsula desenhada fica dentro da região (nunca corta o desenho).
                let cl = (jw - cw) / 2;
                let raio = ch as f64 / 2.0;
                for k in 0..=72 {
                    let ang = (k as f64) * std::f64::consts::PI / 36.0;
                    let (cx, cy) = if ang.cos() >= 0.0 {
                        (cl as f64 + cw as f64 - raio, ct as f64 + raio)
                    } else {
                        (cl as f64 + raio, ct as f64 + raio)
                    };
                    let px = cx + (raio - 0.5) * ang.cos();
                    let py = cy + (raio - 0.5) * ang.sin();
                    assert!(dentro_da_regiao(r, px, py), "{escala} {cap_w}: ({px},{py})");
                }
                // Quinas da janela e vão de cima atravessam o clique.
                let (jwf, jhf) = (jw as f64, jh as f64);
                for (px, py) in [(0.5, 0.5), (0.5, jhf - 0.5), (jwf - 0.5, jhf - 0.5)] {
                    assert!(!dentro_da_regiao(r, px, py));
                }
                assert!(!dentro_da_regiao(
                    r,
                    jwf / 2.0,
                    (ct as f64 - 2.0).max(0.0) - 0.5 + 0.5 * 0.0
                ));
                // Centro pega o clique.
                assert!(dentro_da_regiao(r, jwf / 2.0, ct as f64 + raio));
            }
        }
    }

    /// Controles (medidas da §2.3) ficam a pelo menos 6 DIP da ponta curva.
    #[test]
    fn controles_cabem_dentro_da_capsula() {
        // (rótulo, cápsula w, h, largura do conteúdo, raio do controle da ponta)
        // ociosa: Ditar (12+12+6+28=58) + 8 + divisor 1 + 8 + chip (~59) = 134.
        // gravando aberta: ponto 6 + 8 + barras 51 + 8 + tempo 34 + 8 + cancelar 28 = 143.
        // mãos livres: 28 + 8 + 51 + 8 + 34 + 8 + 28 = 165.
        let casos = [
            ("aberta, ociosa", 184.0, 36.0, 134.0, 14.0),
            ("gravando, aberta", 216.0, 36.0, 143.0, 14.0),
            ("mãos livres", 252.0, 36.0, 165.0, 14.0),
            ("gravando, recolhida", 112.0, 30.0, 6.0 + 8.0 + 51.0, 0.0),
            ("processando, aberta", 196.0, 36.0, 23.0 + 8.0 + 80.0, 0.0),
        ];
        for (rotulo, w, h, conteudo, raio_ctl) in casos {
            let r = h / 2.0;
            let x0 = (w - conteudo) / 2.0;
            // Botão redondo na ponta: margem até a borda >= r - raio (concêntrico, §2.3).
            // Texto, ponto ou barras na ponta: margem >= 14 DIP.
            let minimo = if raio_ctl > 0.0 { r - raio_ctl } else { 14.0 };
            assert!(x0 >= minimo, "{rotulo}: margem {x0:.1} < {minimo:.1}");
        }
    }

    #[test]
    fn protocolo_de_modo_aceita_geometria_exata_e_recusa_absurdos() {
        assert_eq!(interpretar_modo("aberta"), Some((ModoJanela::Aberta, None)));
        assert_eq!(
            interpretar_modo("aberta:188:44:184:36:6:c"),
            Some((
                ModoJanela::Aberta,
                Some(geo(188.0, 44.0, 184.0, 36.0, 6.0, true))
            ))
        );
        assert_eq!(
            interpretar_modo("repouso:72:14:40:5:3:r"),
            Some((
                ModoJanela::Repouso,
                Some(geo(72.0, 14.0, 40.0, 5.0, 3.0, false))
            ))
        );
        for ruim in [
            "lateral",
            "aberta:188:44",
            "aberta:188:44:184:36:6",
            "aberta:188:44:184:36:6:x",
            "aberta:188:44:184:36:6:c:1",
            "aberta:4000:44:184:36:6:c",
            "aberta:188:900:184:36:6:c",
            "aberta:188:44:200:36:6:c",
            "aberta:188:44:184:36:20:c",
            "aberta:NaN:44:184:36:6:c",
            "aberta:inf:44:184:36:6:c",
        ] {
            assert_eq!(interpretar_modo(ruim), None, "{ruim}");
        }
    }

    #[test]
    fn troca_de_modo_pelo_rust_volta_ao_tamanho_padrao() {
        let mut i = interno_teste();
        i.geometria = geo(72.0, 14.0, 56.0, 6.0, 3.0, false); // aceno informado pela página
        let _ = atualizar_modo_e_vigia(&mut i, ModoJanela::Repouso);
        assert_eq!(
            i.geometria.cap_w, 56.0,
            "mesmo modo preserva a geometria da página"
        );
        let _ = atualizar_modo_e_vigia(&mut i, ModoJanela::Compacta);
        assert_eq!(i.geometria, ModoJanela::Compacta.geometria_padrao());
        // Padrões da §2.3: barrinha 40x5 em 72x14 sem recorte; cápsulas com recorte.
        let r = ModoJanela::Repouso.geometria_padrao();
        assert_eq!(
            (r.w, r.h, r.cap_w, r.cap_h, r.recorte_capsula),
            (72.0, 14.0, 40.0, 5.0, false)
        );
        assert!(ModoJanela::Aberta.geometria_padrao().recorte_capsula);
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
            "CreateRoundRectRgn",
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
        assert!(producao.contains("janela::recortar("));
        assert!(producao.contains("posicionar_sem_ativar"));
    }

    /// O Tauri põe hashes na style-src; com hash, o navegador ignora 'unsafe-inline' e descarta
    /// atributos style="" (no Windows real a casca andava meia largura para a direita).
    #[test]
    fn pagina_da_ilha_nao_usa_atributo_style() {
        let html = include_str!("../../pilula/pilula.html");
        assert!(
            !html.contains(" style=\""),
            "pilula.html não pode ter atributo style"
        );
        let js = include_str!("../../pilula/pilula.js");
        assert!(!js.contains("setAttribute(\"style\""));
    }

    /// O ditado processa sem fração conhecida: o segmento da §2.5 (22%, 1,2 s linear) circula
    /// nesse caso; no movimento reduzido, perímetro inteiro parado a 70%.
    #[test]
    fn arco_circula_no_processamento_sem_fracao() {
        let js = include_str!("../../pilula/pilula.js");
        assert!(js.contains("palco.dataset.segmento = \"girando\""));
        assert!(js.contains("const indeterminado = processando && (pct == null"));
        let html = include_str!("../../pilula/pilula.html");
        assert!(html.contains("stroke-dasharray: 0.22 0.78"));
        assert!(html.contains("animation: girar-arco 1.2s linear infinite"));
        assert!(html.contains("--ilha-progresso: #0A84FF"));
        let reduzido = html
            .split("prefers-reduced-motion: reduce")
            .nth(1)
            .unwrap_or("");
        assert!(reduzido.contains("[data-segmento=\"girando\"] .tracos .inteiro { opacity: 0.7; }"));
        assert!(
            reduzido.contains("[data-segmento] .tracos .segmento { animation: none; opacity: 0; }")
        );
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
            saida_ate: None,
            geometria: ModoJanela::Repouso.geometria_padrao(),
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
