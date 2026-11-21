//! Bandeja do sistema e botão da barra de tarefas (spec seção 4).
//!
//! Prioridade de estado na bandeja:
//! erro (10 s) > ouvindo/mãos livres > reunião > transcrevendo > baixando > aviso > pronto.
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tauri::{
    image::Image,
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent},
    window::{ProgressBarState, ProgressBarStatus},
    AppHandle, Manager, Wry,
};

use crate::{caminhos, nativo, pilula::NivelAviso, Casca};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconeBandeja {
    Pronto,
    Ouvindo,
    Transcrevendo,
    Baixando,
    Reuniao,
    Aviso,
    Erro,
}

impl IconeBandeja {
    pub fn bytes(self, barra_clara: bool) -> &'static [u8] {
        match (self, barra_clara) {
            (Self::Pronto, true) => include_bytes!("../icons/bandeja/pronto-barra-clara.ico"),
            (Self::Pronto, false) => include_bytes!("../icons/bandeja/pronto-barra-escura.ico"),
            (Self::Ouvindo, true) => include_bytes!("../icons/bandeja/ouvindo-barra-clara.ico"),
            (Self::Ouvindo, false) => include_bytes!("../icons/bandeja/ouvindo-barra-escura.ico"),
            (Self::Transcrevendo, true) => {
                include_bytes!("../icons/bandeja/transcrevendo-barra-clara.ico")
            }
            (Self::Transcrevendo, false) => {
                include_bytes!("../icons/bandeja/transcrevendo-barra-escura.ico")
            }
            (Self::Baixando, true) => include_bytes!("../icons/bandeja/baixando-barra-clara.ico"),
            (Self::Baixando, false) => include_bytes!("../icons/bandeja/baixando-barra-escura.ico"),
            (Self::Reuniao, true) => include_bytes!("../icons/bandeja/reuniao-barra-clara.ico"),
            (Self::Reuniao, false) => include_bytes!("../icons/bandeja/reuniao-barra-escura.ico"),
            (Self::Aviso, true) => include_bytes!("../icons/bandeja/aviso-barra-clara.ico"),
            (Self::Aviso, false) => include_bytes!("../icons/bandeja/aviso-barra-escura.ico"),
            (Self::Erro, true) => include_bytes!("../icons/bandeja/erro-barra-clara.ico"),
            (Self::Erro, false) => include_bytes!("../icons/bandeja/erro-barra-escura.ico"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaseDitadoBandeja {
    Parado,
    Ouvindo,
    MaosLivres,
    Transcrevendo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaseReuniaoBandeja {
    Nenhuma,
    Gravando,
    Pausada,
    Finalizando,
}

#[derive(Debug, Clone, Default)]
pub struct SnapshotBandeja {
    pub atalho: String,
    pub ditado: Option<FaseDitadoBandeja>,
    pub reuniao: Option<FaseReuniaoBandeja>,
    pub arquivo_progresso: Option<u8>,
    pub arquivo_falhou: bool,
    pub baixando_pct: Option<u8>,
    pub carregando_reconhecedor: bool,
    pub falta_permissao: bool,
    pub erro_recente: Option<(String, Instant)>,
    pub erro_nao_visto: bool,
}

/// Resolve ícone e dica (máximo 127 caracteres) segundo a prioridade da seção 4.1.
pub fn resolver_estado(snap: &SnapshotBandeja, agora: Instant) -> (IconeBandeja, String) {
    let dica = if let Some((ref msg, ate)) = snap.erro_recente {
        if agora < ate {
            return (IconeBandeja::Erro, truncar_dica(&format!("Dita.se: {msg}")));
        }
        String::new()
    } else {
        String::new()
    };
    let _ = dica;

    match snap.ditado.unwrap_or(FaseDitadoBandeja::Parado) {
        FaseDitadoBandeja::Ouvindo => return (IconeBandeja::Ouvindo, "Dita.se: ouvindo".into()),
        FaseDitadoBandeja::MaosLivres => {
            return (
                IconeBandeja::Ouvindo,
                truncar_dica(&format!(
                    "Dita.se: ouvindo em mãos livres. {} para parar",
                    snap.atalho
                )),
            );
        }
        _ => {}
    }

    match snap.reuniao.unwrap_or(FaseReuniaoBandeja::Nenhuma) {
        FaseReuniaoBandeja::Gravando => {
            return (
                IconeBandeja::Reuniao,
                "Dita.se: reunião em andamento".into(),
            );
        }
        FaseReuniaoBandeja::Pausada => {
            return (IconeBandeja::Reuniao, "Dita.se: reunião pausada".into());
        }
        FaseReuniaoBandeja::Finalizando => {
            return (IconeBandeja::Transcrevendo, "Dita.se: transcrevendo".into());
        }
        FaseReuniaoBandeja::Nenhuma => {}
    }

    if snap.ditado == Some(FaseDitadoBandeja::Transcrevendo) {
        return (IconeBandeja::Transcrevendo, "Dita.se: transcrevendo".into());
    }
    if let Some(pct) = snap.arquivo_progresso {
        return (
            IconeBandeja::Transcrevendo,
            truncar_dica(&format!("Dita.se: transcrevendo arquivo · {pct}%")),
        );
    }

    if let Some(pct) = snap.baixando_pct {
        return (
            IconeBandeja::Baixando,
            truncar_dica(&format!("Dita.se: baixando o modelo de voz · {pct}%")),
        );
    }

    if snap.falta_permissao {
        return (
            IconeBandeja::Aviso,
            "Dita.se: falta permissão do microfone".into(),
        );
    }
    if snap.carregando_reconhecedor {
        return (
            IconeBandeja::Aviso,
            "Dita.se: carregando o reconhecedor".into(),
        );
    }

    (
        IconeBandeja::Pronto,
        truncar_dica(&format!(
            "Dita.se: pronto. Segure {} para ditar",
            snap.atalho
        )),
    )
}

fn truncar_dica(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= 127 {
        s.to_string()
    } else {
        let mut out: String = chars[..126].iter().collect();
        out.push('…');
        out
    }
}

/// Estado do progresso e sobreposição no botão da barra de tarefas (seção 4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SobreposicaoBarra {
    Nenhuma,
    Gravando,
    Erro,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressoBarra {
    Nenhum,
    Normal(u64),
    Indeterminado,
    Erro,
}

pub fn resolver_barra_tarefas(snap: &SnapshotBandeja) -> (SobreposicaoBarra, ProgressoBarra) {
    let gravando = matches!(
        snap.ditado,
        Some(FaseDitadoBandeja::Ouvindo | FaseDitadoBandeja::MaosLivres)
    ) || snap.reuniao == Some(FaseReuniaoBandeja::Gravando);

    let sobreposicao = if gravando {
        SobreposicaoBarra::Gravando
    } else if snap.erro_nao_visto {
        SobreposicaoBarra::Erro
    } else {
        SobreposicaoBarra::Nenhuma
    };

    let progresso = if snap.arquivo_falhou {
        ProgressoBarra::Erro
    } else if snap.reuniao == Some(FaseReuniaoBandeja::Finalizando) {
        ProgressoBarra::Indeterminado
    } else if let Some(pct) = snap.arquivo_progresso {
        ProgressoBarra::Normal(pct as u64)
    } else if let Some(pct) = snap.baixando_pct {
        ProgressoBarra::Normal(pct as u64)
    } else {
        ProgressoBarra::Nenhum
    };

    (sobreposicao, progresso)
}

/// Decodifica o melhor quadro (32x32 ou maior) de um arquivo `.ico` (DIB 32-bpp BGRA ou PNG).
pub fn decodificar_ico(bytes: &[u8]) -> Result<Image<'static>, String> {
    if bytes.len() < 6 {
        return Err("ICO curto".into());
    }
    let count = u16::from_le_bytes([bytes[4], bytes[5]]) as usize;
    if count == 0 || bytes.len() < 6 + count * 16 {
        return Err("cabeçalho ICO inválido".into());
    }
    let mut melhor: Option<(usize, usize, usize, usize)> = None;
    for i in 0..count {
        let base = 6 + i * 16;
        let w = if bytes[base] == 0 {
            256
        } else {
            bytes[base] as usize
        };
        let h = if bytes[base + 1] == 0 {
            256
        } else {
            bytes[base + 1] as usize
        };
        let tam = u32::from_le_bytes(bytes[base + 8..base + 12].try_into().unwrap()) as usize;
        let off = u32::from_le_bytes(bytes[base + 12..base + 16].try_into().unwrap()) as usize;
        if off + tam > bytes.len() {
            continue;
        }
        let pontuacao = if w == 32 { 1000 } else { w };
        if melhor.is_none_or(|(p, _, _, _)| pontuacao > p) {
            melhor = Some((pontuacao, w, h, off));
        }
    }
    let Some((_, w, h, off)) = melhor else {
        return Err("nenhuma entrada válida no ICO".into());
    };
    let slice = &bytes[off..];
    if slice.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Image::from_bytes(slice)
            .map_err(|e| e.to_string())
            .map(|img| img.to_owned());
    }
    if slice.len() < 40 {
        return Err("DIB curto".into());
    }
    let hdr = u32::from_le_bytes(slice[0..4].try_into().unwrap()) as usize;
    let bpp = u16::from_le_bytes(slice[14..16].try_into().unwrap());
    if bpp != 32 || slice.len() < hdr + w * h * 4 {
        return Err(format!("formato DIB não suportado (bpp={bpp})"));
    }
    let pixels = &slice[hdr..hdr + w * h * 4];
    let mut rgba = vec![0u8; w * h * 4];
    for y in 0..h {
        let src_y = h - 1 - y;
        for x in 0..w {
            let s = (src_y * w + x) * 4;
            let d = (y * w + x) * 4;
            rgba[d] = pixels[s + 2];
            rgba[d + 1] = pixels[s + 1];
            rgba[d + 2] = pixels[s];
            rgba[d + 3] = pixels[s + 3];
        }
    }
    Ok(Image::new_owned(rgba, w as u32, h as u32))
}

/// Gera um ícone 16x16 RGBA para sobreposição na barra de tarefas (ponto vermelho ou erro).
#[cfg(windows)]
fn imagem_sobreposicao(erro: bool) -> Image<'static> {
    let mut rgba = vec![0u8; 16 * 16 * 4];
    for y in 0..16i32 {
        for x in 0..16i32 {
            let dx = x as f32 - 7.5;
            let dy = y as f32 - 7.5;
            let d = (dx * dx + dy * dy).sqrt();
            let idx = ((y * 16 + x) * 4) as usize;
            if d <= 6.2 {
                // Vermelho Windows (#C42B1C)
                rgba[idx] = 0xC4;
                rgba[idx + 1] = 0x2B;
                rgba[idx + 2] = 0x1C;
                rgba[idx + 3] = 255;
            } else if d <= 7.4 {
                // Contorno branco para legibilidade sobre qualquer barra
                rgba[idx] = 255;
                rgba[idx + 1] = 255;
                rgba[idx + 2] = 255;
                rgba[idx + 3] = 235;
            }
            if erro && (7..=8).contains(&x) && ((4..=8).contains(&y) || (10..=11).contains(&y)) {
                rgba[idx] = 255;
                rgba[idx + 1] = 255;
                rgba[idx + 2] = 255;
                rgba[idx + 3] = 255;
            }
        }
    }
    Image::new_owned(rgba, 16, 16)
}

pub struct Bandeja {
    app: AppHandle,
    icone: TrayIcon<Wry>,
    item_ditar: MenuItem<Wry>,
    item_cancelar: MenuItem<Wry>,
    item_aba: CheckMenuItem<Wry>,
    item_iniciar: CheckMenuItem<Wry>,
    estado: Mutex<SnapshotBandeja>,
    ultimo_icone: Mutex<Option<(IconeBandeja, bool)>>,
}

impl Bandeja {
    pub fn criar(
        app: &AppHandle,
        iniciar: bool,
        aba_visivel: bool,
        atalho: &str,
    ) -> tauri::Result<Arc<Self>> {
        let abrir = MenuItem::with_id(app, "abrir", "Abrir Dita.se", true, None::<&str>)?;
        let item_ditar =
            MenuItem::with_id(app, "ditar", "Ditar em mãos livres", true, None::<&str>)?;
        let item_cancelar =
            MenuItem::with_id(app, "cancelar", "Cancelar ditado", false, None::<&str>)?;
        let sep1 = PredefinedMenuItem::separator(app)?;
        let item_aba = CheckMenuItem::with_id(
            app,
            "aba",
            "Mostrar a aba na borda da tela",
            true,
            aba_visivel,
            None::<&str>,
        )?;
        let item_iniciar = CheckMenuItem::with_id(
            app,
            "iniciar",
            "Iniciar com o Windows",
            true,
            iniciar,
            None::<&str>,
        )?;
        let relatorio = MenuItem::with_id(
            app,
            "relatorio",
            "Copiar relatório de teste",
            true,
            None::<&str>,
        )?;
        let atualizar =
            MenuItem::with_id(app, "atualizar", "Procurar atualização", true, None::<&str>)?;
        let sep2 = PredefinedMenuItem::separator(app)?;
        let sair = MenuItem::with_id(app, "sair", "Sair", true, None::<&str>)?;

        let menu = Menu::with_items(
            app,
            &[
                &abrir,
                &item_ditar,
                &item_cancelar,
                &sep1,
                &item_aba,
                &item_iniciar,
                &relatorio,
                &atualizar,
                &sep2,
                &sair,
            ],
        )?;

        let barra_clara = nativo::sistema::barra_clara();
        let mut construtor = TrayIconBuilder::with_id("bandeja")
            .tooltip(truncar_dica(&format!(
                "Dita.se: pronto. Segure {atalho} para ditar"
            )))
            .menu(&menu)
            .show_menu_on_left_click(false);
        if let Ok(img) = decodificar_ico(IconeBandeja::Pronto.bytes(barra_clara)) {
            construtor = construtor.icon(img);
        } else if let Some(icone) = app.default_window_icon() {
            construtor = construtor.icon(icone.clone());
        }

        let icone = construtor
            .on_menu_event(move |app, evento| {
                let Some(casca) = app.try_state::<Arc<Casca>>().map(|s| s.inner().clone()) else {
                    return;
                };
                match evento.id.as_ref() {
                    "abrir" => crate::mostrar_principal(app),
                    "ditar" => {
                        let c = casca.controlador.clone();
                        std::thread::spawn(move || c.alternar());
                    }
                    "cancelar" => {
                        let c = casca.controlador.clone();
                        std::thread::spawn(move || {
                            if let Some(a) = c.atalho.get() {
                                a.enviar(crate::atalho::Entrada::EncerradoPorFora);
                            }
                            c.cancelar();
                        });
                    }
                    "aba" => {
                        let ligar = !casca.prefs.pilula_sempre_visivel();
                        casca
                            .prefs
                            .definir("pilulaSempreVisivel", if ligar { "1" } else { "0" });
                        casca.controlador.preferencia_mudou("pilulaSempreVisivel");
                        crate::ponte::emitir(app, "ajustes");
                    }
                    "iniciar" => {
                        let ligar = !casca.prefs.iniciar_com_windows();
                        casca
                            .prefs
                            .definir("iniciarComWindows", if ligar { "1" } else { "0" });
                        crate::iniciar_com_windows(app, ligar);
                        crate::ponte::emitir(app, "ajustes");
                    }
                    "relatorio" => {
                        let texto = crate::relatorio::montar(&casca);
                        let (aviso, nivel) = if nativo::colagem::copiar(&texto).is_ok() {
                            ("Relatório copiado. Cole no chat", NivelAviso::Info)
                        } else {
                            ("Não consegui copiar o relatório", NivelAviso::Erro)
                        };
                        casca.pilula.avisar(aviso, 3.0, nivel);
                    }
                    "atualizar" => {
                        let a = app.clone();
                        tauri::async_runtime::spawn(async move {
                            crate::procurar_atualizacao(a, true).await
                        });
                    }
                    "sair" => {
                        crate::desligar_tudo(&casca);
                        app.exit(0);
                    }
                    _ => {}
                }
            })
            .on_tray_icon_event(|bandeja, evento| {
                if let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } = evento
                {
                    crate::mostrar_principal(bandeja.app_handle());
                }
            })
            .build(app)?;

        Ok(Arc::new(Self {
            app: app.clone(),
            icone,
            item_ditar,
            item_cancelar,
            item_aba,
            item_iniciar,
            estado: Mutex::new(SnapshotBandeja {
                atalho: atalho.to_string(),
                ..Default::default()
            }),
            ultimo_icone: Mutex::new(Some((IconeBandeja::Pronto, barra_clara))),
        }))
    }

    pub fn atualizar_atalho(&self, simbolo: &str) {
        self.estado.lock().unwrap().atalho = simbolo.to_string();
        self.aplicar();
    }

    pub fn atualizar_aba_marcada(&self, visivel: bool) {
        let _ = self.item_aba.set_checked(visivel);
    }

    pub fn atualizar_iniciar_marcado(&self, ligar: bool) {
        let _ = self.item_iniciar.set_checked(ligar);
    }

    pub fn definir_ditado(&self, fase: FaseDitadoBandeja) {
        self.estado.lock().unwrap().ditado = Some(fase);
        let ditando = matches!(
            fase,
            FaseDitadoBandeja::Ouvindo | FaseDitadoBandeja::MaosLivres
        );
        let _ = self.item_ditar.set_text(if ditando {
            "Parar e colar"
        } else {
            "Ditar em mãos livres"
        });
        let _ = self.item_cancelar.set_enabled(ditando);
        self.aplicar();
    }

    pub fn definir_baixando(&self, pct: Option<u8>) {
        self.estado.lock().unwrap().baixando_pct = pct;
        self.aplicar();
    }

    pub fn definir_carregando(&self, carregando: bool) {
        self.estado.lock().unwrap().carregando_reconhecedor = carregando;
        self.aplicar();
    }

    pub fn definir_falta_permissao(&self, falta: bool) {
        let mudou = {
            let mut s = self.estado.lock().unwrap();
            let antes = s.falta_permissao;
            s.falta_permissao = falta;
            antes != falta
        };
        if mudou {
            self.aplicar();
        }
    }

    pub fn definir_reuniao(&self, fase: FaseReuniaoBandeja) {
        self.estado.lock().unwrap().reuniao = Some(fase);
        self.aplicar();
    }

    pub fn definir_arquivo(&self, progresso: Option<u8>, falhou: bool) {
        let mut s = self.estado.lock().unwrap();
        s.arquivo_progresso = progresso;
        if falhou {
            s.arquivo_falhou = true;
            s.erro_nao_visto = true;
        }
        drop(s);
        self.aplicar();
    }

    pub fn registrar_erro(self: &Arc<Self>, texto: &str) {
        let ate = Instant::now() + Duration::from_secs(10);
        {
            let mut s = self.estado.lock().unwrap();
            s.erro_recente = Some((texto.to_string(), ate));
            s.erro_nao_visto = true;
        }
        self.aplicar();
        let eu = self.clone();
        let _ = std::thread::Builder::new()
            .name("ditase.bandeja.erro".into())
            .spawn(move || {
                std::thread::sleep(Duration::from_secs(10) + Duration::from_millis(50));
                eu.aplicar();
            });
    }

    pub fn janela_aberta(&self) {
        let mudou = {
            let mut s = self.estado.lock().unwrap();
            let antes = s.erro_nao_visto || s.arquivo_falhou;
            s.erro_nao_visto = false;
            s.arquivo_falhou = false;
            antes
        };
        if mudou {
            self.aplicar();
        }
    }

    pub fn aplicar(&self) {
        let snap = self.estado.lock().unwrap().clone();
        let (icone, dica) = resolver_estado(&snap, Instant::now());
        let barra_clara = nativo::sistema::barra_clara();

        let trocar_icone = {
            let mut ult = self.ultimo_icone.lock().unwrap();
            if *ult != Some((icone, barra_clara)) {
                *ult = Some((icone, barra_clara));
                true
            } else {
                false
            }
        };
        if trocar_icone {
            match decodificar_ico(icone.bytes(barra_clara)) {
                Ok(img) => {
                    let _ = self.icone.set_icon(Some(img));
                }
                Err(e) => caminhos::nota(format!("bandeja: ícone inválido ({e})")),
            }
        }
        let _ = self.icone.set_tooltip(Some(&dica));

        // Botão da janela principal na barra de tarefas (seção 4.2).
        if let Some(janela) = self.app.get_webview_window("principal") {
            let (sobreposicao, progresso) = resolver_barra_tarefas(&snap);
            #[cfg(windows)]
            {
                let img = match sobreposicao {
                    SobreposicaoBarra::Nenhuma => None,
                    SobreposicaoBarra::Gravando => Some(imagem_sobreposicao(false)),
                    SobreposicaoBarra::Erro => Some(imagem_sobreposicao(true)),
                };
                let _ = janela.set_overlay_icon(img);
            }
            #[cfg(not(windows))]
            let _ = sobreposicao;

            let estado_barra = match progresso {
                ProgressoBarra::Nenhum => ProgressBarState {
                    status: Some(ProgressBarStatus::None),
                    progress: None,
                },
                ProgressoBarra::Normal(p) => ProgressBarState {
                    status: Some(ProgressBarStatus::Normal),
                    progress: Some(p),
                },
                ProgressoBarra::Indeterminado => ProgressBarState {
                    status: Some(ProgressBarStatus::Indeterminate),
                    progress: None,
                },
                ProgressoBarra::Erro => ProgressBarState {
                    status: Some(ProgressBarStatus::Error),
                    progress: Some(100),
                },
            };
            let _ = janela.set_progress_bar(estado_barra);
        }
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn prioridade_da_bandeja_segue_spec() {
        let agora = Instant::now();
        let mut s = SnapshotBandeja {
            atalho: "Ctrl dir.".into(),
            ..Default::default()
        };
        assert_eq!(
            resolver_estado(&s, agora),
            (
                IconeBandeja::Pronto,
                "Dita.se: pronto. Segure Ctrl dir. para ditar".into()
            )
        );

        s.carregando_reconhecedor = true;
        assert_eq!(resolver_estado(&s, agora).0, IconeBandeja::Aviso);

        s.falta_permissao = true;
        assert_eq!(
            resolver_estado(&s, agora),
            (
                IconeBandeja::Aviso,
                "Dita.se: falta permissão do microfone".into()
            )
        );
        s.falta_permissao = false;

        s.baixando_pct = Some(42);
        assert_eq!(
            resolver_estado(&s, agora),
            (
                IconeBandeja::Baixando,
                "Dita.se: baixando o modelo de voz · 42%".into()
            )
        );

        s.arquivo_progresso = Some(48);
        assert_eq!(
            resolver_estado(&s, agora),
            (
                IconeBandeja::Transcrevendo,
                "Dita.se: transcrevendo arquivo · 48%".into()
            )
        );

        s.reuniao = Some(FaseReuniaoBandeja::Gravando);
        assert_eq!(resolver_estado(&s, agora).0, IconeBandeja::Reuniao);

        s.ditado = Some(FaseDitadoBandeja::MaosLivres);
        assert_eq!(
            resolver_estado(&s, agora),
            (
                IconeBandeja::Ouvindo,
                "Dita.se: ouvindo em mãos livres. Ctrl dir. para parar".into()
            )
        );

        s.erro_recente = Some((
            "Microfone indisponível".into(),
            agora + Duration::from_secs(5),
        ));
        assert_eq!(
            resolver_estado(&s, agora),
            (IconeBandeja::Erro, "Dita.se: Microfone indisponível".into())
        );

        // Passados os 10 s, volta ao estado real (mãos livres).
        assert_eq!(
            resolver_estado(&s, agora + Duration::from_secs(11)).0,
            IconeBandeja::Ouvindo
        );
    }

    #[test]
    fn barra_de_tarefas_nao_usa_progresso_para_ditado_curto() {
        let mut s = SnapshotBandeja {
            ditado: Some(FaseDitadoBandeja::Ouvindo),
            ..Default::default()
        };
        assert_eq!(
            resolver_barra_tarefas(&s),
            (SobreposicaoBarra::Gravando, ProgressoBarra::Nenhum)
        );
        s.ditado = Some(FaseDitadoBandeja::Transcrevendo);
        assert_eq!(
            resolver_barra_tarefas(&s),
            (SobreposicaoBarra::Nenhuma, ProgressoBarra::Nenhum)
        );
        s.arquivo_progresso = Some(55);
        assert_eq!(
            resolver_barra_tarefas(&s),
            (SobreposicaoBarra::Nenhuma, ProgressoBarra::Normal(55))
        );
        s.reuniao = Some(FaseReuniaoBandeja::Finalizando);
        assert_eq!(
            resolver_barra_tarefas(&s),
            (SobreposicaoBarra::Nenhuma, ProgressoBarra::Indeterminado)
        );
        s.arquivo_falhou = true;
        s.erro_nao_visto = true;
        assert_eq!(
            resolver_barra_tarefas(&s),
            (SobreposicaoBarra::Erro, ProgressoBarra::Erro)
        );
    }

    #[test]
    fn todos_os_14_icones_embutidos_carregam() {
        for ic in [
            IconeBandeja::Pronto,
            IconeBandeja::Ouvindo,
            IconeBandeja::Transcrevendo,
            IconeBandeja::Baixando,
            IconeBandeja::Reuniao,
            IconeBandeja::Aviso,
            IconeBandeja::Erro,
        ] {
            for claro in [true, false] {
                let img = decodificar_ico(ic.bytes(claro));
                assert!(img.is_ok(), "falhou ao ler ICO {:?} claro={}", ic, claro);
            }
        }
    }
}
