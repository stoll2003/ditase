//! Camada nativa. No Windows, cada módulo fala com o Win32; fora dele (testes e `cargo check`
//! no Mac) há substitutos inertes com a mesma assinatura. Nada aqui guarda teclas ou texto.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub enum Insercao {
    Colado,
    /// Não deu para colar: o texto ficou copiado (fora do histórico do Windows) e a pílula avisa.
    SoCopiado,
    /// Nem colou nem copiou (marcas de exclusão recusadas): o texto só está no histórico do Dita.se.
    Recusado,
}

#[cfg(windows)]
#[path = "win_colagem.rs"]
pub mod colagem;
#[cfg(windows)]
#[path = "win_gancho.rs"]
pub mod gancho;
#[cfg(windows)]
#[path = "win_janela.rs"]
pub mod janela;
#[cfg(windows)]
#[path = "win_midia.rs"]
pub mod midia;
#[cfg(windows)]
#[path = "win_sistema.rs"]
pub mod sistema;

#[cfg(not(windows))]
pub mod gancho {
    use crate::atalho::{Entrada, Visao};
    use std::{
        sync::{mpsc::Sender, Arc, Mutex},
        time::Instant,
    };
    /// Sem gancho fora do Windows.
    pub fn instalar(
        _visao: Arc<Mutex<Visao>>,
        _emissor: Sender<Entrada>,
        _relogio: Instant,
    ) -> bool {
        false
    }
}

#[cfg(not(windows))]
pub mod colagem {
    use super::Insercao;
    pub fn copiar(_texto: &str) -> Result<(), String> {
        Err("área de transferência só no Windows".into())
    }
    pub fn inserir(_texto: &str) -> Insercao {
        Insercao::SoCopiado
    }
}

#[cfg(not(windows))]
#[allow(dead_code)]
pub mod janela {
    use std::sync::Arc;
    pub fn sem_foco(_hwnd: isize) {}
    pub fn posicionar_sem_ativar(_hwnd: isize, _x: i32, _y: i32, _w: i32, _h: i32, _mostrar: bool) {
    }
    pub fn esconder(_hwnd: isize) {}
    pub fn tela_cheia_ativa() -> bool {
        false
    }
    pub fn cursor_fora_da_janela(_hwnd: isize) -> bool {
        false
    }
    pub fn observar_tela_cheia(_cb: Arc<dyn Fn(bool) + Send + Sync>) {}
}

#[cfg(not(windows))]
pub mod midia {
    pub fn decodificar(_caminho: &std::path::Path) -> Result<Vec<f32>, String> {
        Err("Media Foundation só no Windows".into())
    }
}

#[cfg(not(windows))]
#[allow(dead_code)]
pub mod sistema {
    pub fn versao() -> String {
        format!("{} (fora do Windows)", std::env::consts::OS)
    }
    pub fn barra_clara() -> bool {
        false
    }
    pub fn tema_app_escuro() -> bool {
        false
    }
    pub fn suporta_mica() -> bool {
        false
    }
    /// Programa em primeiro plano: (nome do executável, nome para exibir). Nunca o título da janela.
    pub fn app_em_foco() -> (String, String) {
        (String::new(), String::new())
    }
    pub fn abrir(alvo: &str) -> std::io::Result<()> {
        std::process::Command::new("open")
            .arg(alvo)
            .spawn()
            .map(|_| ())
    }
    /// Espaço livre no volume; fora do Windows o baixador segue sem a conferência.
    pub fn alerta(titulo: &str, texto: &str) {
        eprintln!("{titulo}: {texto}");
    }
    pub fn espaco_livre(_pasta: &std::path::Path) -> Option<u64> {
        None
    }
}
