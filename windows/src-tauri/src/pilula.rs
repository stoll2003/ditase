//! Aba na borda direita do Windows (spec v1.0.1, seção 3 e MOTION-windows.md):
//! segunda janela Tauri transparente, sempre no topo, fora da barra de tarefas e sem nunca
//! roubar o foco (`WS_EX_NOACTIVATE`, posicionamento e exibição só por `SetWindowPos` com
//! `SWP_NOACTIVATE`, nunca `WebviewWindow::show()`).
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
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

/// Tamanhos lógicos da janela por modo (MOTION-windows.md §6 e SPEC-windows.md §3.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModoJanela {
    Repouso,
    Compacta,
    Aberta,
}

impl ModoJanela {
    pub fn dimensoes_logicas(self) -> (f64, f64) {
        match self {
            ModoJanela::Repouso => (6.0, 64.0),
            ModoJanela::Compacta => (44.0, 64.0),
            ModoJanela::Aberta => (256.0, 80.0),
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
    pub fn direita(&self) -> i32 {
        self.x + self.w
    }
    pub fn work_direita(&self) -> i32 {
        self.work_x + self.work_w
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PosicaoAba {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub espelhada: bool,
}

/// Escolhe monitor, borda e coordenadas físicas da aba segundo a seção 3.4 da spec:
/// 1. Borda direita do monitor principal se nenhum monitor encosta à direita na faixa vertical da aba.
/// 2. Senão, borda direita do monitor com a maior borda direita.
/// 3. Se a barra de tarefas estiver à direita (work_direita < direita), espelha na borda esquerda
///    do monitor mais à esquerda.
///
/// Altura: centro a 62% da altura da área de trabalho.
pub fn escolher_borda(monitores: &[RetanguloMonitor], modo: ModoJanela) -> Option<PosicaoAba> {
    if monitores.is_empty() {
        return None;
    }
    let principal = monitores
        .iter()
        .find(|m| m.principal)
        .copied()
        .unwrap_or(monitores[0]);

    let (lw, lh) = modo.dimensoes_logicas();
    let calcular_y = |m: &RetanguloMonitor, h_phys: i32| -> i32 {
        let centro = m.work_y as f64 + (m.work_h as f64) * 0.62;
        let y = (centro - (h_phys as f64) / 2.0).round() as i32;
        y.clamp(m.work_y, (m.work_y + m.work_h - h_phys).max(m.work_y))
    };

    // Verifica se algum monitor encosta à direita do principal na faixa de altura da aba.
    let h_principal = (lh * principal.escala).round() as i32;
    let y_principal = calcular_y(&principal, h_principal);
    let faixa_topo = y_principal;
    let faixa_base = y_principal + h_principal;

    let encosta_a_direita = monitores.iter().any(|m| {
        if m.x == principal.x && m.y == principal.y && m.w == principal.w && m.h == principal.h {
            return false;
        }
        let toca_x = (m.x - principal.direita()).abs() <= 4;
        let sobrepoe_y = m.y < faixa_base && (m.y + m.h) > faixa_topo;
        toca_x && sobrepoe_y
    });

    let escolhido = if !encosta_a_direita {
        principal
    } else {
        monitores
            .iter()
            .copied()
            .max_by_key(|m| m.work_direita())
            .unwrap_or(principal)
    };

    // Regra 3: barra de tarefas à direita (área de trabalho termina antes do monitor).
    if escolhido.work_direita() < escolhido.direita() {
        let mais_esquerda = monitores
            .iter()
            .copied()
            .min_by_key(|m| m.work_x)
            .unwrap_or(escolhido);
        let w_phys = (lw * mais_esquerda.escala).round() as i32;
        let h_phys = (lh * mais_esquerda.escala).round() as i32;
        let y = calcular_y(&mais_esquerda, h_phys);
        return Some(PosicaoAba {
            x: mais_esquerda.work_x,
            y,
            w: w_phys,
            h: h_phys,
            espelhada: true,
        });
    }

    let w_phys = (lw * escolhido.escala).round() as i32;
    let h_phys = (lh * escolhido.escala).round() as i32;
    let x = escolhido.work_direita() - w_phys;
    let y = calcular_y(&escolhido, h_phys);
    Some(PosicaoAba {
        x,
        y,
        w: w_phys,
        h: h_phys,
        espelhada: false,
    })
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

pub fn msg_borda(espelhada: bool) -> Value {
    json!({"tipo": "borda", "lado": if espelhada { "esquerda" } else { "direita" }})
}

struct Interno {
    fase: Fase,
    modo: ModoJanela,
    sempre_visivel: bool,
    tela_cheia: bool,
    baixando_modelo: Option<(u64, u64)>,
    aviso_ate: Option<Instant>,
    espelhada: bool,
    ultimo_nivel: Instant,
    atalho: String,
    canal: Option<Channel<Value>>,
    vigia_cursor_ativo: Arc<AtomicBool>,
}

pub struct Pilula {
    app: AppHandle,
    interno: Mutex<Interno>,
}

impl Pilula {
    pub fn criar(app: &AppHandle, sempre_visivel: bool, atalho: &str) -> Arc<Self> {
        let (lw, lh) = ModoJanela::Repouso.dimensoes_logicas();
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

        if let Err(e) = &janela {
            crate::caminhos::nota(format!("pílula: não abriu ({e})"));
        }
        #[cfg(windows)]
        if let Ok(ref j) = janela {
            if let Ok(h) = j.hwnd() {
                crate::nativo::janela::sem_foco(h.0 as isize);
            }
        }

        let p = Arc::new(Self {
            app: app.clone(),
            interno: Mutex::new(Interno {
                fase: Fase::Ociosa,
                modo: ModoJanela::Repouso,
                sempre_visivel,
                tela_cheia: crate::nativo::janela::tela_cheia_ativa(),
                baixando_modelo: None,
                aviso_ate: None,
                espelhada: false,
                ultimo_nivel: Instant::now(),
                atalho: atalho.to_string(),
                canal: None,
                vigia_cursor_ativo: Arc::new(AtomicBool::new(false)),
            }),
        });

        // Recalcular posição quando a escala do monitor mudar (seção 3.4).
        if let Ok(j) = janela {
            let p_escala = p.clone();
            j.on_window_event(move |ev| {
                if let tauri::WindowEvent::ScaleFactorChanged { .. } = ev {
                    p_escala.aplicar_janela();
                }
            });
        }

        // Observar tela cheia por EVENT_SYSTEM_FOREGROUND (zero polling em repouso).
        let p_tela = p.clone();
        crate::nativo::janela::observar_tela_cheia(Arc::new(move |ativa| {
            p_tela.definir_tela_cheia(ativa);
        }));

        p.enviar(json!({"tipo": "atalho", "simbolo": atalho}));
        p.mostrar(Fase::Ociosa);
        p
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

    /// A página da pílula carregou e entregou o canal IPC dela: guarda e reenvia atalho, borda, modelo e fase.
    pub fn conectar(&self, canal: Channel<Value>) {
        self.interno.lock().unwrap().canal = Some(canal);
        self.reenviar();
    }

    /// Reenvia o estado completo para a página da aba.
    pub fn reenviar(&self) {
        let (fase, atalho, espelhada, baixando) = {
            let i = self.interno.lock().unwrap();
            (i.fase, i.atalho.clone(), i.espelhada, i.baixando_modelo)
        };
        self.enviar(json!({"tipo": "atalho", "simbolo": atalho}));
        self.enviar(msg_borda(espelhada));
        if let Some((feito, total)) = baixando {
            self.enviar(msg_modelo(Some(feito), total));
        }
        self.enviar(msg_fase(fase));
    }

    fn deve_estar_visivel(i: &Interno) -> bool {
        let aviso_ativo = i.aviso_ate.is_some_and(|ate| Instant::now() < ate);
        let em_atividade = i.fase != Fase::Ociosa || aviso_ativo || i.baixando_modelo.is_some();
        if em_atividade {
            true
        } else {
            i.sempre_visivel && !i.tela_cheia
        }
    }

    /// Posiciona, redimensiona e mostra/esconde a janela sem nunca ativar (`SWP_NOACTIVATE`).
    pub fn aplicar_janela(&self) {
        let Some(j) = self.janela() else {
            return;
        };
        let (visivel, modo) = {
            let i = self.interno.lock().unwrap();
            (Self::deve_estar_visivel(&i), i.modo)
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
        let Some(pos) = escolher_borda(&monitores, modo) else {
            return;
        };
        let mudou_borda = {
            let mut i = self.interno.lock().unwrap();
            let antes = i.espelhada;
            i.espelhada = pos.espelhada;
            antes != pos.espelhada
        };
        if mudou_borda {
            self.enviar(msg_borda(pos.espelhada));
        }
        #[cfg(windows)]
        if let Ok(h) = j.hwnd() {
            crate::nativo::janela::posicionar_sem_ativar(
                h.0 as isize,
                pos.x,
                pos.y,
                pos.w,
                pos.h,
                true,
            );
        }
        #[cfg(not(windows))]
        let _ = pos;
    }

    pub fn definir_modo(&self, modo: ModoJanela) {
        let flag = {
            let mut i = self.interno.lock().unwrap();
            if i.modo == modo {
                return;
            }
            i.modo = modo;
            i.vigia_cursor_ativo.clone()
        };
        self.aplicar_janela();

        // Fallback de saída do ponteiro (MOTION-windows.md §6): 10 Hz só enquanto aberta, nunca em repouso.
        if modo == ModoJanela::Aberta {
            if !flag.swap(true, Ordering::AcqRel) {
                let app = self.app.clone();
                let flag_thread = flag.clone();
                let _ = std::thread::Builder::new()
                    .name("ditase.aba.cursor".into())
                    .spawn(move || {
                        while flag_thread.load(Ordering::Acquire) {
                            std::thread::sleep(Duration::from_millis(100));
                            if !flag_thread.load(Ordering::Acquire) {
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
        } else {
            flag.store(false, Ordering::Release);
        }
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
        {
            let mut i = self.interno.lock().unwrap();
            i.fase = fase;
            if fase != Fase::Ociosa {
                i.aviso_ate = None;
            }
            i.modo = match fase {
                Fase::Ociosa => {
                    if i.baixando_modelo.is_some() {
                        ModoJanela::Compacta
                    } else {
                        ModoJanela::Repouso
                    }
                }
                Fase::Gravando | Fase::Processando => ModoJanela::Compacta,
                Fase::MaosLivres => ModoJanela::Aberta,
            };
        }
        self.aplicar_janela();
        self.enviar(msg_fase(fase));
    }

    /// Mensagem curta com nível (`info` ou `erro`); a aba volta sozinha a ociosa depois de `segundos`.
    pub fn avisar(&self, texto: &str, segundos: f64, nivel: NivelAviso) {
        let duracao = match nivel {
            NivelAviso::Erro => segundos.max(3.5),
            NivelAviso::Info => segundos,
        };
        {
            let mut i = self.interno.lock().unwrap();
            i.fase = Fase::Ociosa;
            i.modo = ModoJanela::Aberta;
            i.aviso_ate = Some(Instant::now() + Duration::from_secs_f64(duracao + 0.4));
        }
        self.aplicar_janela();
        self.enviar(msg_aviso(texto, duracao, nivel));
        if nivel == NivelAviso::Erro {
            if let Some(casca) = self.app.try_state::<Arc<crate::Casca>>() {
                if let Some(b) = casca.bandeja.get() {
                    b.registrar_erro(texto);
                }
            }
        }

        let app = self.app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs_f64(duracao + 0.5));
            if let Some(estado) = app.try_state::<Arc<crate::Casca>>() {
                let mut i = estado.pilula.interno.lock().unwrap();
                if i.aviso_ate.is_some_and(|ate| Instant::now() >= ate) {
                    i.aviso_ate = None;
                    if i.fase == Fase::Ociosa {
                        i.modo = if i.baixando_modelo.is_some() {
                            ModoJanela::Compacta
                        } else {
                            ModoJanela::Repouso
                        };
                    }
                    drop(i);
                    estado.pilula.aplicar_janela();
                }
            }
        });
    }

    /// Progresso do download do modelo de voz (`feito: None` encerra o estado de download).
    pub fn modelo(&self, feito: Option<u64>, total: u64) {
        {
            let mut i = self.interno.lock().unwrap();
            i.baixando_modelo = feito.map(|f| (f, total));
            if i.fase == Fase::Ociosa && i.aviso_ate.is_none() {
                i.modo = if i.baixando_modelo.is_some() {
                    ModoJanela::Compacta
                } else {
                    ModoJanela::Repouso
                };
            }
        }
        self.aplicar_janela();
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
    fn monitor_unico_posiciona_na_borda_direita_a_62_por_cento_em_varias_escalas() {
        for escala in [1.0, 1.25, 1.5, 1.75, 2.0] {
            let m = monitor(0, 0, 1920, 1080, escala, true);
            for modo in [
                ModoJanela::Repouso,
                ModoJanela::Compacta,
                ModoJanela::Aberta,
            ] {
                let pos = escolher_borda(&[m], modo).unwrap();
                assert!(!pos.espelhada);
                // Sem folga de 1 px: x + w == borda direita física.
                assert_eq!(pos.x + pos.w, m.work_direita());
                let esperado_y = ((m.work_h as f64) * 0.62 - (pos.h as f64) / 2.0).round() as i32;
                assert_eq!(pos.y, esperado_y);
            }
        }
    }

    #[test]
    fn dois_monitores_escolhe_parede_mais_a_direita() {
        let principal = monitor(0, 0, 1920, 1080, 1.0, true);
        let direita = monitor(1920, 0, 2560, 1440, 1.5, false);
        let pos = escolher_borda(&[principal, direita], ModoJanela::Repouso).unwrap();
        assert!(!pos.espelhada);
        assert_eq!(pos.x + pos.w, 1920 + 2560);
    }

    #[test]
    fn monitor_a_esquerda_mantem_principal() {
        let esquerda = monitor(-1920, 0, 1920, 1080, 1.0, false);
        let principal = monitor(0, 0, 1920, 1080, 1.0, true);
        let pos = escolher_borda(&[esquerda, principal], ModoJanela::Repouso).unwrap();
        assert!(!pos.espelhada);
        assert_eq!(pos.x + pos.w, 1920);
    }

    #[test]
    fn barra_de_tarefas_a_direita_espelha_para_borda_esquerda() {
        let mut principal = monitor(0, 0, 1920, 1080, 1.0, true);
        principal.work_w = 1860; // barra de tarefas à direita (60 px)
        principal.work_h = 1080;
        let pos = escolher_borda(&[principal], ModoJanela::Aberta).unwrap();
        assert!(pos.espelhada);
        assert_eq!(pos.x, 0);
    }

    #[test]
    fn pilula_nunca_chama_show_do_tauri() {
        let codigo = include_str!("pilula.rs");
        let producao = codigo.split("#[cfg(test)]").next().unwrap_or(codigo);
        assert!(
            !producao.contains(".show()"),
            "pilula.rs não pode chamar WebviewWindow::show(), que pode ativar a janela"
        );
    }
}
