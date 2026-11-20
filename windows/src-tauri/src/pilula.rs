//! Pílula (porta de `Pilula.swift` e `PilulaProgresso.swift`): segunda janela Tauri,
//! transparente, sempre no topo, fora da barra de tarefas e sem roubar o foco. A página
//! `windows/pilula/pilula.html` desenha; aqui só mandamos o estado com `window.pilula(...)`.
use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

use serde_json::{json, Value};
use tauri::{AppHandle, Manager, PhysicalPosition, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

pub const ROTULO: &str = "pilula";
const LARGURA: f64 = 260.0;
const ALTURA: f64 = 64.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fase { Ociosa, Gravando, MaosLivres, Processando }

impl Fase {
    fn id(self) -> &'static str {
        match self { Fase::Ociosa => "ociosa", Fase::Gravando => "gravando", Fase::MaosLivres => "maosLivres", Fase::Processando => "processando" }
    }
}

struct Interno { fase: Fase, sempre_visivel: bool, ultimo_nivel: Instant }

pub struct Pilula { app: AppHandle, interno: Mutex<Interno> }

impl Pilula {
    pub fn criar(app: &AppHandle, sempre_visivel: bool, atalho: &str) -> Self {
        let janela = WebviewWindowBuilder::new(app, ROTULO, WebviewUrl::App("pilula.html".into()))
            .title("Dita.se")
            .inner_size(LARGURA, ALTURA)
            .resizable(false)
            .decorations(false)
            .transparent(true)
            .shadow(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .focused(false)
            .visible(false)
            .build();
        match janela {
            Ok(j) => {
                #[cfg(windows)]
                if let Ok(h) = j.hwnd() { crate::nativo::janela::sem_foco(h.0 as isize); }
                posicionar(&j);
            }
            Err(e) => crate::caminhos::nota(format!("pílula: não abriu ({e})")),
        }
        let p = Self { app: app.clone(), interno: Mutex::new(Interno { fase: Fase::Ociosa, sempre_visivel, ultimo_nivel: Instant::now() }) };
        p.enviar(json!({"tipo": "atalho", "simbolo": atalho}));
        p.mostrar(Fase::Ociosa);
        p
    }

    fn janela(&self) -> Option<WebviewWindow> { self.app.get_webview_window(ROTULO) }

    fn enviar(&self, mensagem: Value) {
        if let Some(j) = self.janela() {
            let _ = j.eval(format!("window.pilula && window.pilula({mensagem})"));
        }
    }

    fn visibilidade(&self, fase: Fase) {
        let sempre = self.interno.lock().unwrap().sempre_visivel;
        if let Some(j) = self.janela() {
            if fase == Fase::Ociosa && !sempre { let _ = j.hide(); } else { let _ = j.show(); }
        }
    }

    pub fn fase(&self) -> Fase { self.interno.lock().unwrap().fase }

    pub fn mostrar(&self, fase: Fase) {
        self.interno.lock().unwrap().fase = fase;
        self.enviar(json!({"tipo": "fase", "fase": fase.id()}));
        self.visibilidade(fase);
    }

    /// Mensagem curta; a pílula volta sozinha a ociosa depois de `segundos`.
    pub fn avisar(&self, texto: &str, segundos: f64) {
        self.interno.lock().unwrap().fase = Fase::Ociosa;
        self.enviar(json!({"tipo": "aviso", "texto": texto, "segundos": segundos}));
        if let Some(j) = self.janela() { let _ = j.show(); }
        let sempre = self.interno.lock().unwrap().sempre_visivel;
        if !sempre {
            let app = self.app.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs_f64(segundos + 0.3));
                if let Some(estado) = app.try_state::<std::sync::Arc<crate::Casca>>() {
                    if estado.pilula.fase() == Fase::Ociosa { estado.pilula.visibilidade(Fase::Ociosa); }
                }
            });
        }
    }

    /// Nível do microfone, no máximo a cada 50 ms (como no Mac).
    pub fn empurrar_nivel(&self, nivel: f32) {
        {
            let mut i = self.interno.lock().unwrap();
            if i.ultimo_nivel.elapsed() < Duration::from_millis(50) { return; }
            i.ultimo_nivel = Instant::now();
        }
        self.enviar(json!({"tipo": "nivel", "nivel": nivel}));
    }

    pub fn segundos(&self, s: u64) { self.enviar(json!({"tipo": "segundos", "segundos": s})); }
    pub fn atalho(&self, simbolo: &str) { self.enviar(json!({"tipo": "atalho", "simbolo": simbolo})); }

    pub fn sempre_visivel(&self, sim: bool) {
        let fase = { let mut i = self.interno.lock().unwrap(); i.sempre_visivel = sim; i.fase };
        self.visibilidade(fase);
    }

    /// Progresso de arquivo ou reunião (`ProgressoDaPilula`); `None` encerra.
    pub fn progresso(&self, evento: Option<Value>) { self.enviar(json!({"tipo": "progresso", "progresso": evento})); }
}

/// Centro inferior do monitor principal, acima da barra de tarefas.
fn posicionar(j: &WebviewWindow) {
    let Ok(Some(monitor)) = j.primary_monitor() else { return };
    let area = monitor.work_area();
    let escala = monitor.scale_factor();
    let largura = (LARGURA * escala) as i32;
    let altura = (ALTURA * escala) as i32;
    let x = area.position.x + (area.size.width as i32 - largura) / 2;
    let y = area.position.y + area.size.height as i32 - altura - (16.0 * escala) as i32;
    let _ = j.set_position(PhysicalPosition::new(x, y));
}
