//! Dita.se para Windows: casca Tauri 2 (WebView2) servindo a mesma interface do Mac.
//! A interface fala pelo comando `ponte` (contrato em `docs/contrato-ponte-lancamento.md`).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod atalho;
mod audio;
mod caminhos;
mod controlador;
mod decodificacao;
mod integracao;
mod lancamento;
mod motores_locais;
mod nativo;
mod pilula;
mod ponte;
mod preferencias;
mod relatorio;
mod teste;

use std::{path::PathBuf, sync::Arc, time::Duration};

use serde_json::{json, Value};
use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, DragDropEvent, Manager, RunEvent, State, WebviewUrl, WebviewWindowBuilder, WindowEvent,
};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};
use tauri_plugin_updater::UpdaterExt;

use crate::{controlador::Controlador, integracao::Integracao, lancamento::{CapturaDoSistema, Lancamento}, motores_locais::MotoresLocais, pilula::Pilula, preferencias::Preferencias};

const CANAL_DE_TESTE: &str = "https://github.com/stoll2003/ditase/releases/download/windows-teste/latest.json";
const CHAVE_DO_CANAL: &str = env!("DITASE_UPDATER_PUB");

/// Estado do app compartilhado pelos comandos.
pub struct Casca {
    pub app: AppHandle,
    pub prefs: Arc<Preferencias>,
    pub integracao: Arc<Integracao>,
    pub lancamento: Arc<Lancamento>,
    pub pilula: Arc<Pilula>,
    pub controlador: Arc<Controlador>,
}

#[tauri::command]
async fn ponte(casca: State<'_, Arc<Casca>>, metodo: String, params: Option<Value>) -> Result<String, String> {
    let casca = casca.inner().clone();
    let params = params.unwrap_or_else(|| json!({}));
    tauri::async_runtime::spawn_blocking(move || ponte::atender(&casca, &metodo, &params))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
fn pilula_acao(casca: State<'_, Arc<Casca>>, acao: String) {
    if acao == "pronta" { casca.pilula.reenviar(); return; }
    let c = casca.controlador.clone();
    std::thread::spawn(move || match acao.as_str() {
        "alternar" => c.alternar(),
        "parar" => { if let Some(a) = c.atalho.get() { a.enviar(atalho::Entrada::EncerradoPorFora); } c.terminar(true) }
        "cancelar" => { if let Some(a) = c.atalho.get() { a.enviar(atalho::Entrada::EncerradoPorFora); } c.cancelar() }
        _ => {}
    });
}

/// As janelas só mostram a interface empacotada: qualquer navegação para fora é negada (links
/// abrem no navegador do sistema pela ponte, nunca dentro do app).
pub fn navegacao_permitida(url: &tauri::Url) -> bool {
    match url.scheme() {
        "tauri" => url.host_str() == Some("localhost"),
        "http" | "https" => url.host_str() == Some("tauri.localhost"),
        _ => false,
    }
}

pub fn iniciar_com_windows(app: &AppHandle, ligar: bool) {
    let auto = app.autolaunch();
    let r = if ligar { auto.enable() } else { auto.disable() };
    if let Err(e) = r { caminhos::nota(format!("iniciar com o Windows: {e}")); }
}

/// Saída do app (bandeja, reinício do atualizador ou fim pelo Windows), uma vez só: o ditado em
/// curso grava o que falta, o coletor da reunião grava o que estava em memória, e o servidor de IA
/// para. O que não terminou volta pela recuperação na próxima abertura. Teto total de cerca de 2 s.
pub fn desligar_tudo(casca: &Casca) {
    static FEITO: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if FEITO.swap(true, std::sync::atomic::Ordering::AcqRel) { return; }
    let inicio = std::time::Instant::now();
    casca.controlador.desligar(Duration::from_millis(1200));
    casca.lancamento.desligar(Duration::from_millis(2000).saturating_sub(inicio.elapsed()).max(Duration::from_millis(500)));
    casca.integracao.motores.desligar();
    caminhos::nota(format!("saída: desligado em {:.2} s", inicio.elapsed().as_secs_f64()));
}

fn mostrar_principal(app: &AppHandle) {
    if let Some(j) = app.get_webview_window("principal") {
        let _ = j.unminimize();
        let _ = j.show();
        let _ = j.set_focus();
    }
}

fn abrir_principal(app: &AppHandle) -> tauri::Result<()> {
    let janela = WebviewWindowBuilder::new(app, "principal", WebviewUrl::App("index.html".into()))
        .title("Dita.se")
        .inner_size(1120.0, 760.0)
        .min_inner_size(820.0, 560.0)
        .on_navigation(navegacao_permitida)
        .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
        .build()?;
    #[cfg(windows)]
    if let Ok(h) = janela.hwnd() { nativo::colagem::definir_dono(h.0 as isize); }
    let app2 = app.clone();
    janela.on_window_event(move |evento| match evento {
        // Fechar a janela só a esconde: o atalho e a pílula continuam. Sair pela bandeja.
        WindowEvent::CloseRequested { api, .. } => {
            api.prevent_close();
            if let Some(j) = app2.get_webview_window("principal") { let _ = j.hide(); }
        }
        // O arraste nativo entrega o caminho real (o WebKit do Mac só entregava o nome).
        WindowEvent::DragDrop(DragDropEvent::Drop { paths, .. }) => soltar(&app2, paths.clone()),
        _ => {}
    });
    Ok(())
}

fn soltar(app: &AppHandle, caminhos_soltos: Vec<PathBuf>) {
    let Some(casca) = app.try_state::<Arc<Casca>>().map(|s| s.inner().clone()) else { return };
    let app = app.clone();
    std::thread::spawn(move || {
        let aceitos: Vec<PathBuf> = caminhos_soltos.into_iter().filter(|p| p.is_file() && decodificacao::suportado(p)).collect();
        if aceitos.is_empty() { return; }
        match casca.lancamento.adicionar(&aceitos) {
            Ok(Some(id)) => ponte::emitir_evento(&app, "ditado:arquivos-soltos", &json!(id)),
            Ok(None) => {}
            Err(e) => caminhos::nota(format!("arraste: {e}")),
        }
    });
}

fn bandeja(app: &AppHandle, iniciar: bool) -> tauri::Result<()> {
    let abrir = MenuItem::with_id(app, "abrir", "Abrir Dita.se", true, None::<&str>)?;
    let ditar = MenuItem::with_id(app, "ditar", "Começar ou parar ditado", true, None::<&str>)?;
    let auto = CheckMenuItem::with_id(app, "iniciar", "Iniciar com o Windows", true, iniciar, None::<&str>)?;
    let relatorio = MenuItem::with_id(app, "relatorio", "Copiar relatório de teste", true, None::<&str>)?;
    let atualizar = MenuItem::with_id(app, "atualizar", "Procurar atualização", true, None::<&str>)?;
    let sair = MenuItem::with_id(app, "sair", "Sair", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&abrir, &ditar, &sep, &auto, &relatorio, &atualizar, &sep, &sair])?;
    let mut construtor = TrayIconBuilder::with_id("bandeja").tooltip("Dita.se").menu(&menu).show_menu_on_left_click(false);
    if let Some(icone) = app.default_window_icon() { construtor = construtor.icon(icone.clone()); }
    construtor
        .on_menu_event(move |app, evento| {
            let Some(casca) = app.try_state::<Arc<Casca>>().map(|s| s.inner().clone()) else { return };
            match evento.id.as_ref() {
                "abrir" => mostrar_principal(app),
                "ditar" => { let c = casca.controlador.clone(); std::thread::spawn(move || c.alternar()); }
                "iniciar" => {
                    let ligar = !casca.prefs.iniciar_com_windows();
                    casca.prefs.definir("iniciarComWindows", if ligar { "1" } else { "0" });
                    iniciar_com_windows(app, ligar);
                }
                "relatorio" => {
                    let texto = relatorio::montar(&casca);
                    let aviso = if nativo::colagem::copiar(&texto).is_ok() { "Relatório copiado. Cole no chat" } else { "Não consegui copiar o relatório" };
                    casca.pilula.avisar(aviso, 3.0);
                }
                "atualizar" => { let a = app.clone(); tauri::async_runtime::spawn(async move { procurar_atualizacao(a, true).await }); }
                "sair" => { desligar_tudo(&casca); app.exit(0); }
                _ => {}
            }
        })
        .on_tray_icon_event(|bandeja, evento| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = evento { mostrar_principal(bandeja.app_handle()); }
        })
        .build(app)?;
    Ok(())
}

/// Canal de teste: `latest.json` da release `windows-teste`, assinado com a chave de `windows/updater.pub`.
async fn procurar_atualizacao(app: AppHandle, manual: bool) {
    if CHAVE_DO_CANAL.is_empty() {
        caminhos::nota("atualização: sem chave pública embutida (windows/updater.pub)");
        return;
    }
    let resultado = async {
        let url = tauri::Url::parse(CANAL_DE_TESTE)?;
        let atualizador = app.updater_builder().pubkey(CHAVE_DO_CANAL).endpoints(vec![url])?.build()?;
        let Some(nova) = atualizador.check().await? else { return Ok::<_, Box<dyn std::error::Error + Send + Sync>>(None) };
        let pergunta = format!("A versão de teste {} está disponível (você tem {}). Instalar agora? O Dita.se reinicia.", nova.version, nova.current_version);
        let sim = app.dialog().message(pergunta).title("Atualização do Dita.se")
            .buttons(MessageDialogButtons::OkCancelCustom("Instalar".into(), "Depois".into())).blocking_show();
        if !sim { return Ok(Some(false)); }
        nova.download_and_install(|_, _| {}, || {}).await?;
        Ok(Some(true))
    }.await;
    match resultado {
        Ok(Some(true)) => {
            caminhos::nota("atualização: instalada; reiniciando");
            if let Some(c) = app.try_state::<Arc<Casca>>() { desligar_tudo(&c); }
            app.restart();
        }
        Ok(Some(false)) => caminhos::nota("atualização: adiada"),
        Ok(None) => {
            caminhos::nota("atualização: já é a mais recente");
            if manual { if let Some(c) = app.try_state::<Arc<Casca>>() { c.pilula.avisar("Você já tem a versão mais recente", 2.5); } }
        }
        Err(e) => caminhos::nota(format!("atualização: {e}")),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Err(e) = caminhos::preparar() {
        nativo::sistema::alerta("Dita.se", &e);
        std::process::exit(1);
    }
    if teste::rodar(&args) { return; }
    caminhos::nota(format!("abrindo Dita.se {}", env!("CARGO_PKG_VERSION")));
    tauri::Builder::default()
        // Primeiro plugin: uma segunda abertura só traz a janela da primeira para frente.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| mostrar_principal(app)))
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, Some(vec!["--bandeja"])))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![ponte, pilula_acao])
        .setup(|app| {
            let inicio = std::time::Instant::now();
            let handle = app.handle().clone();
            let prefs = Arc::new(Preferencias::abrir(caminhos::preferencias()));
            let estado = handle.clone();
            let motores = MotoresLocais::novo(caminhos::modelos(), prefs.clone(), Arc::new(move || ponte::emitir(&estado, "estado")));
            let integracao = Arc::new(Integracao::abrir(&caminhos::banco(), motores.clone()).map_err(|e| format!("banco: {e}"))?.com_pasta_de_trabalho(caminhos::trabalho().join("ditado")));
            let pilula = Arc::new(Pilula::criar(&handle, prefs.pilula_sempre_visivel(), prefs.atalho().simbolo()));
            let avisar = handle.clone();
            let controlador = Controlador::new(prefs.clone(), integracao.clone(), pilula.clone(), Arc::new(controlador::SistemaReal), move |assunto| ponte::emitir(&avisar, assunto));
            let (eventos, pilula_eventos) = (handle.clone(), pilula.clone());
            let avisar_lancamento: lancamento::Avisar = Arc::new(move |assunto, detalhe| match detalhe {
                None => ponte::emitir(&eventos, assunto),
                Some(v) => {
                    // A pílula mostra a etapa de arquivo ou reunião quando não há ditado.
                    if assunto == "ditado:progresso" { pilula_eventos.progresso(Some(v.clone())); }
                    if assunto == "ditado:resultado" { pilula_eventos.progresso(None); }
                    ponte::emitir_evento(&eventos, assunto, &v);
                }
            });
            let dicionario = integracao.clone();
            let preparar: lancamento::Preparar = Arc::new(move |texto| dicionario.preparar(texto));
            let microfone = prefs.clone();
            let capturador = Arc::new(CapturaDoSistema { microfone: Arc::new(move || microfone.microfone()) });
            let lancamento = Lancamento::new(&caminhos::banco(), &caminhos::audio_de_reuniao(), motores.clone(), capturador, preparar, avisar_lancamento)
                .map_err(|e| format!("arquivos e reuniões: {e}"))?;
            controlador.compartilhar_microfone(lancamento.clone());
            let casca = Arc::new(Casca { app: handle.clone(), prefs: prefs.clone(), integracao, lancamento, pilula, controlador: controlador.clone() });
            motores.preparar_fala();
            app.manage(casca);
            caminhos::nota(format!("abertura: serviços prontos em {} ms", inicio.elapsed().as_millis()));
            abrir_principal(&handle)?;
            caminhos::nota(format!("abertura: janela criada em {} ms", inicio.elapsed().as_millis()));
            // Aberto pelo "iniciar com o Windows": fica só na bandeja e na pílula.
            if std::env::args().any(|a| a == "--bandeja") { if let Some(j) = handle.get_webview_window("principal") { let _ = j.hide(); } }
            bandeja(&handle, prefs.iniciar_com_windows())?;
            controlador.ligar();
            let a = handle.clone();
            tauri::async_runtime::spawn(async move {
                tokio_espera(Duration::from_secs(20)).await;
                procurar_atualizacao(a, false).await;
            });
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("Dita.se não abriu")
        .run(|app, evento| {
            // Fim pelo Windows (desligar o PC, sair da sessão) ou qualquer saída que não passou pela bandeja.
            if let RunEvent::Exit = evento {
                if let Some(c) = app.try_state::<Arc<Casca>>() { desligar_tudo(&c); }
            }
        });
}

async fn tokio_espera(d: Duration) { let _ = tauri::async_runtime::spawn_blocking(move || std::thread::sleep(d)).await; }
