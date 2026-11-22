//! Dita.se para Windows: casca Tauri 2 (WebView2) servindo a mesma interface do Mac.
//! A interface fala pelo comando `ponte` (contrato em `docs/contrato-ponte-lancamento.md`).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod atalho;
mod audio;
mod bandeja;
mod caminhos;
mod controlador;
mod decodificacao;
#[cfg(test)]
mod e2e;
mod emenda;
mod integracao;
mod lancamento;
mod motores_locais;
mod nativo;
mod pilula;
mod ponte;
mod preferencias;
mod relatorio;
mod teste;

use std::{
    path::PathBuf,
    sync::{Arc, OnceLock},
    time::Duration,
};

use serde_json::{json, Value};
use tauri::{
    window::Color, AppHandle, DragDropEvent, Manager, RunEvent, State, WebviewUrl,
    WebviewWindowBuilder, WindowEvent,
};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};
use tauri_plugin_updater::UpdaterExt;

use crate::{
    bandeja::{Bandeja, FaseDitadoBandeja, FaseReuniaoBandeja},
    controlador::Controlador,
    integracao::Integracao,
    lancamento::{CapturaDoSistema, Lancamento},
    motores_locais::MotoresLocais,
    pilula::{NivelAviso, Pilula},
    preferencias::Preferencias,
};

const CANAL_ESTAVEL: &str =
    "https://github.com/stoll2003/ditase/releases/latest/download/latest.json";
const CHAVE_DO_CANAL: &str = env!("DITASE_UPDATER_PUB");

/// Estado do app compartilhado pelos comandos.
pub struct Casca {
    pub app: AppHandle,
    pub prefs: Arc<Preferencias>,
    pub integracao: Arc<Integracao>,
    pub lancamento: Arc<Lancamento>,
    pub pilula: Arc<Pilula>,
    pub controlador: Arc<Controlador>,
    pub bandeja: OnceLock<Arc<Bandeja>>,
}

#[tauri::command]
async fn ponte(
    casca: State<'_, Arc<Casca>>,
    metodo: String,
    params: Option<Value>,
) -> Result<String, String> {
    let casca = casca.inner().clone();
    let params = params.unwrap_or_else(|| json!({}));
    tauri::async_runtime::spawn_blocking(move || ponte::atender(&casca, &metodo, &params))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
fn pilula_conectar(casca: State<'_, Arc<Casca>>, canal: tauri::ipc::Channel<Value>) {
    casca.pilula.conectar(canal);
}

#[tauri::command]
fn pilula_acao(casca: State<'_, Arc<Casca>>, acao: String) {
    if acao == "pronta" {
        casca.pilula.reenviar();
        return;
    }
    if let Some(nome_modo) = acao.strip_prefix("modo:") {
        if let Some((modo, geometria)) = pilula::interpretar_modo(nome_modo) {
            casca.pilula.definir_modo(modo, geometria);
        }
        return;
    }
    let c = casca.controlador.clone();
    std::thread::spawn(move || match acao.as_str() {
        "alternar" => c.alternar(),
        "parar" => {
            if let Some(a) = c.atalho.get() {
                a.enviar(atalho::Entrada::EncerradoPorFora);
            }
            c.terminar(true)
        }
        "cancelar" => {
            if let Some(a) = c.atalho.get() {
                a.enviar(atalho::Entrada::EncerradoPorFora);
            }
            c.cancelar()
        }
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
    if let Err(e) = r {
        caminhos::nota(format!("iniciar com o Windows: {e}"));
    }
}

/// Saída do app (bandeja, reinício do atualizador ou fim pelo Windows), uma vez só: o ditado em
/// curso grava o que falta, o coletor da reunião grava o que estava em memória, e o servidor de IA
/// para. O que não terminou volta pela recuperação na próxima abertura. Teto total de cerca de 2 s.
pub fn desligar_tudo(casca: &Casca) {
    static FEITO: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if FEITO.swap(true, std::sync::atomic::Ordering::AcqRel) {
        return;
    }
    let inicio = std::time::Instant::now();
    casca.controlador.desligar(Duration::from_millis(1200));
    casca.lancamento.desligar(
        Duration::from_millis(2000)
            .saturating_sub(inicio.elapsed())
            .max(Duration::from_millis(500)),
    );
    casca.integracao.motores.desligar();
    caminhos::nota(format!(
        "saída: desligado em {:.2} s",
        inicio.elapsed().as_secs_f64()
    ));
}

pub fn mostrar_principal(app: &AppHandle) {
    if let Some(j) = app.get_webview_window("principal") {
        let _ = j.unminimize();
        let _ = j.show();
        let _ = j.set_focus();
    }
    if let Some(c) = app.try_state::<Arc<Casca>>() {
        if let Some(b) = c.bandeja.get() {
            b.janela_aberta();
        }
    }
}

fn cores_da_janela(escuro: bool) -> (Color, Color) {
    if escuro {
        (Color(0x19, 0x19, 0x19, 0xFF), Color(0xF5, 0xF5, 0xF5, 0xFF))
    } else {
        (Color(0xED, 0xED, 0xEF, 0xFF), Color(0x17, 0x17, 0x17, 0xFF))
    }
}

fn abrir_principal(app: &AppHandle) -> tauri::Result<()> {
    let (fundo, texto) = cores_da_janela(nativo::sistema::tema_app_escuro());
    let janela = WebviewWindowBuilder::new(app, "principal", WebviewUrl::App("index.html".into()))
        .title("Dita.se")
        .inner_size(1120.0, 760.0)
        .min_inner_size(820.0, 560.0)
        .background_color(fundo)
        .on_navigation(navegacao_permitida)
        .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
        .build()?;
    nativo::janela::pintar_barra_de_titulo(&janela, fundo, texto);
    #[cfg(windows)]
    if let Ok(h) = janela.hwnd() {
        nativo::colagem::definir_dono(h.0 as isize);
    }
    let app2 = app.clone();
    janela.on_window_event(move |evento| match evento {
        // Fechar a janela só a esconde: o atalho e a pílula continuam. Sair pela bandeja.
        WindowEvent::CloseRequested { api, .. } => {
            api.prevent_close();
            if let Some(j) = app2.get_webview_window("principal") {
                let _ = j.hide();
            }
        }
        WindowEvent::Focused(true) => {
            // A permissão pode ter mudado enquanto os Ajustes do Windows estavam em foco.
            ponte::emitir(&app2, "estado");
            if let Some(c) = app2.try_state::<Arc<Casca>>() {
                if let Some(b) = c.bandeja.get() {
                    b.janela_aberta();
                }
            }
        }
        WindowEvent::ThemeChanged(tema) => {
            if let Some(j) = app2.get_webview_window("principal") {
                let (fundo, texto) = cores_da_janela(*tema == tauri::Theme::Dark);
                let _ = j.set_background_color(Some(fundo));
                nativo::janela::pintar_barra_de_titulo(&j, fundo, texto);
            }
        }
        // O arraste nativo entrega o caminho real (o WebKit do Mac só entregava o nome).
        WindowEvent::DragDrop(DragDropEvent::Drop { paths, .. }) => soltar(&app2, paths.clone()),
        _ => {}
    });
    Ok(())
}

fn soltar(app: &AppHandle, caminhos_soltos: Vec<PathBuf>) {
    let Some(casca) = app.try_state::<Arc<Casca>>().map(|s| s.inner().clone()) else {
        return;
    };
    let app = app.clone();
    std::thread::spawn(move || {
        let aceitos: Vec<PathBuf> = caminhos_soltos
            .into_iter()
            .filter(|p| p.is_file() && decodificacao::suportado(p))
            .collect();
        if aceitos.is_empty() {
            return;
        }
        match casca.lancamento.adicionar(&aceitos) {
            Ok(Some(id)) => ponte::emitir_evento(&app, "ditado:arquivos-soltos", &json!(id)),
            Ok(None) => {}
            Err(e) => caminhos::nota(format!("arraste: {e}")),
        }
    });
}

/// Canal estável: `latest.json` da última release, assinado com a chave de `windows/updater.pub`.
pub async fn procurar_atualizacao(app: AppHandle, manual: bool) {
    if CHAVE_DO_CANAL.is_empty() {
        caminhos::nota("atualização: sem chave pública embutida (windows/updater.pub)");
        return;
    }
    let resultado = async {
        let url = tauri::Url::parse(CANAL_ESTAVEL)?;
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
            if let Some(c) = app.try_state::<Arc<Casca>>() {
                desligar_tudo(&c);
            }
            app.restart();
        }
        Ok(Some(false)) => caminhos::nota("atualização: adiada"),
        Ok(None) => {
            caminhos::nota("atualização: já é a mais recente");
            if manual {
                if let Some(c) = app.try_state::<Arc<Casca>>() {
                    c.pilula
                        .avisar("Você já tem a versão mais recente", 2.5, NivelAviso::Info);
                }
            }
        }
        Err(e) => caminhos::nota(format!("atualização: {e}")),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if teste::rodar(&args) {
        return;
    }
    tauri::Builder::default()
        // Primeiro plugin: uma segunda abertura só traz a janela da primeira para frente.
        // A migração de caminhos roda só em `setup`, depois que a instância única está garantida (M-1).
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            mostrar_principal(app)
        }))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--bandeja"]),
        ))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            ponte,
            pilula_acao,
            pilula_conectar
        ])
        .setup(|app| {
            if let Err(e) = caminhos::preparar() {
                nativo::sistema::alerta("Dita.se", &e);
                std::process::exit(1);
            }
            caminhos::nota(format!("abrindo Dita.se {}", env!("CARGO_PKG_VERSION")));
            let inicio = std::time::Instant::now();
            let handle = app.handle().clone();
            let prefs = Arc::new(Preferencias::abrir(caminhos::preferencias()));
            let estado = handle.clone();
            let transicao_fala = Arc::new(std::sync::Mutex::new(TransicaoFala::default()));
            let transicao_fala_cb = transicao_fala.clone();
            let motores = MotoresLocais::novo(
                caminhos::modelos(),
                prefs.clone(),
                Arc::new(move || {
                    ponte::emitir(&estado, "estado");
                    if let Some(c) = estado.try_state::<Arc<Casca>>() {
                        let (baixando, carregando, pronto) = c.integracao.motores.resumo_fala();
                        let acao = transicao_fala_cb
                            .lock()
                            .unwrap()
                            .avancar(baixando, carregando, pronto);
                        if let Some((feito, total)) = baixando {
                            c.pilula.modelo(Some(feito), total);
                            let pct = feito
                                .saturating_mul(100)
                                .checked_div(total)
                                .map_or(0, |p| p.min(100) as u8);
                            if let Some(b) = c.bandeja.get() {
                                b.definir_baixando(Some(pct));
                                b.definir_carregando(false);
                            }
                        } else {
                            if acao.encerrar_modelo {
                                c.pilula.modelo(None, 0);
                            }
                            if let Some(b) = c.bandeja.get() {
                                b.definir_baixando(None);
                                b.definir_carregando(carregando);
                            }
                            if acao.avisar_pronto {
                                c.pilula.avisar("Pronto para ditar", 2.5, NivelAviso::Info);
                            }
                        }
                    }
                }),
            );
            let integracao = Arc::new(
                Integracao::abrir(&caminhos::banco(), motores.clone())
                    .map_err(|e| format!("banco: {e}"))?
                    .com_pasta_de_trabalho(caminhos::trabalho().join("ditado")),
            );
            let pilula = Pilula::criar(
                &handle,
                prefs.pilula_sempre_visivel(),
                prefs.atalho().simbolo(),
            );
            let avisar = handle.clone();
            let controlador = Controlador::new(
                prefs.clone(),
                integracao.clone(),
                pilula.clone(),
                Arc::new(controlador::SistemaReal),
                move |assunto| {
                    ponte::emitir(&avisar, assunto);
                    if assunto == "estado" {
                        if let Some(c) = avisar.try_state::<Arc<Casca>>() {
                            if let Some(b) = c.bandeja.get() {
                                let fase = match c.controlador.fase() {
                                    controlador::Fase::Parado => FaseDitadoBandeja::Parado,
                                    controlador::Fase::Gravando => {
                                        if c.controlador.maos_livres() {
                                            FaseDitadoBandeja::MaosLivres
                                        } else {
                                            FaseDitadoBandeja::Ouvindo
                                        }
                                    }
                                    controlador::Fase::Processando => {
                                        FaseDitadoBandeja::Transcrevendo
                                    }
                                };
                                b.definir_ditado(fase);
                                b.definir_falta_permissao(!audio::microfone_disponivel());
                            }
                        }
                    }
                },
            );
            let (eventos, pilula_eventos) = (handle.clone(), pilula.clone());
            let avisar_lancamento: lancamento::Avisar =
                Arc::new(move |assunto, detalhe| match detalhe {
                    None => {
                        ponte::emitir(&eventos, assunto);
                        if let Some(c) = eventos.try_state::<Arc<Casca>>() {
                            if let Some(b) = c.bandeja.get() {
                                if let Ok(est) = c.lancamento.estado(true) {
                                    let reuniao = match est
                                        .get("reuniao")
                                        .and_then(|r| r.get("fase"))
                                        .and_then(Value::as_str)
                                    {
                                        Some("gravando") => FaseReuniaoBandeja::Gravando,
                                        Some("pausada") => FaseReuniaoBandeja::Pausada,
                                        Some("finalizando") => FaseReuniaoBandeja::Finalizando,
                                        _ => FaseReuniaoBandeja::Nenhuma,
                                    };
                                    b.definir_reuniao(reuniao);
                                }
                            }
                        }
                    }
                    Some(v) => {
                        // A pílula mostra a etapa de arquivo ou reunião quando não há ditado.
                        if assunto == "ditado:progresso" {
                            pilula_eventos.progresso(Some(v.clone()));
                            if let Some(c) = eventos.try_state::<Arc<Casca>>() {
                                if let Some(b) = c.bandeja.get() {
                                    let pct = v
                                        .get("progresso")
                                        .and_then(Value::as_f64)
                                        .map(|p| (p * 100.0).round().clamp(0.0, 100.0) as u8);
                                    b.definir_arquivo(pct, false);
                                }
                            }
                        }
                        if assunto == "ditado:resultado" {
                            pilula_eventos.progresso(None);
                            if let Some(c) = eventos.try_state::<Arc<Casca>>() {
                                if let Some(b) = c.bandeja.get() {
                                    let falhou = v.get("erro").is_some();
                                    b.definir_arquivo(None, falhou);
                                }
                            }
                        }
                        ponte::emitir_evento(&eventos, assunto, &v);
                    }
                });
            let dicionario = integracao.clone();
            let preparar: lancamento::Preparar = Arc::new(move |texto| dicionario.preparar(texto));
            let microfone = prefs.clone();
            let capturador = Arc::new(CapturaDoSistema {
                microfone: Arc::new(move || microfone.microfone()),
            });
            let lancamento = Lancamento::new(
                &caminhos::banco(),
                &caminhos::audio_de_reuniao(),
                motores.clone(),
                capturador,
                preparar,
                avisar_lancamento,
            )
            .map_err(|e| format!("arquivos e reuniões: {e}"))?;
            controlador.compartilhar_microfone(lancamento.clone());
            let casca = Arc::new(Casca {
                app: handle.clone(),
                prefs: prefs.clone(),
                integracao,
                lancamento,
                pilula,
                controlador: controlador.clone(),
                bandeja: OnceLock::new(),
            });
            let b = Bandeja::criar(
                &handle,
                prefs.iniciar_com_windows(),
                prefs.pilula_sempre_visivel(),
                prefs.atalho().simbolo(),
            )?;
            b.definir_falta_permissao(!audio::microfone_disponivel());
            let _ = casca.bandeja.set(b);
            app.manage(casca);
            motores.preparar_fala();
            caminhos::nota(format!(
                "abertura: serviços prontos em {} ms",
                inicio.elapsed().as_millis()
            ));
            abrir_principal(&handle)?;
            caminhos::nota(format!(
                "abertura: janela criada em {} ms",
                inicio.elapsed().as_millis()
            ));
            // Aberto pelo "iniciar com o Windows": fica só na bandeja e na pílula.
            if std::env::args().any(|a| a == "--bandeja") {
                if let Some(j) = handle.get_webview_window("principal") {
                    let _ = j.hide();
                }
            }
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
                if let Some(c) = app.try_state::<Arc<Casca>>() {
                    desligar_tudo(&c);
                }
            }
        });
}

async fn tokio_espera(d: Duration) {
    let _ = tauri::async_runtime::spawn_blocking(move || std::thread::sleep(d)).await;
}

/// Rastreia a transição `Baixando -> Carregando -> Pronto` do modelo de fala (M-1):
/// `avisar_quando_pronto` sobrevive à etapa intermediária `Carregando` para disparar
/// `"Pronto para ditar"` ao chegar em `Pronto`.
#[derive(Default)]
struct TransicaoFala {
    baixando: bool,
    avisar_quando_pronto: bool,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct AcaoFala {
    encerrar_modelo: bool,
    avisar_pronto: bool,
}

impl TransicaoFala {
    fn avancar(
        &mut self,
        baixando: Option<(u64, u64)>,
        carregando: bool,
        pronto: bool,
    ) -> AcaoFala {
        if baixando.is_some() {
            self.baixando = true;
            self.avisar_quando_pronto = true;
            return AcaoFala::default();
        }
        let encerrar_modelo = std::mem::replace(&mut self.baixando, false);
        let avisar_pronto = if pronto {
            std::mem::replace(&mut self.avisar_quando_pronto, false)
        } else if !carregando {
            self.avisar_quando_pronto = false;
            false
        } else {
            false
        };
        AcaoFala {
            encerrar_modelo,
            avisar_pronto,
        }
    }
}

#[cfg(test)]
mod testes_main {
    use super::*;

    #[test]
    fn transicao_fala_avisa_pronto_apos_baixando_carregando_pronto() {
        let mut t = TransicaoFala::default();
        assert_eq!(
            t.avancar(Some((50, 100)), false, false),
            AcaoFala::default()
        );
        // Entra em Carregando: encerra barra de download mas preserva o aviso pendente.
        assert_eq!(
            t.avancar(None, true, false),
            AcaoFala {
                encerrar_modelo: true,
                avisar_pronto: false,
            }
        );
        // Chega em Pronto: dispara "Pronto para ditar" uma única vez.
        assert_eq!(
            t.avancar(None, false, true),
            AcaoFala {
                encerrar_modelo: false,
                avisar_pronto: true,
            }
        );
        assert_eq!(t.avancar(None, false, true), AcaoFala::default());
    }

    #[test]
    fn transicao_fala_nao_avisa_sem_download_previo() {
        let mut t = TransicaoFala::default();
        assert_eq!(t.avancar(None, true, false), AcaoFala::default());
        assert_eq!(t.avancar(None, false, true), AcaoFala::default());
    }
}
