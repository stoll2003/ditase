//! Atalho global do ditado (porta de `Atalho.swift`), sem nada do Windows: testável no Mac.
//!
//! - Segurar a tecla: grava enquanto segura, transcreve ao soltar.
//! - Dois toques rápidos, ou Espaço com a tecla segurada: mãos livres. Um novo toque encerra.
//! - Esc cancela a gravação. Ctrl+Alt+V cola de novo o último ditado.
//!
//! O gancho (`nativo::gancho`) só traduz teclas em [`Entrada`] e decide se engole a tecla
//! olhando a [`Visao`]; quem decide o gesto é a [`Maquina`], numa thread própria.
use std::{
    sync::{mpsc, Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

/// Soltar antes disso é toque, não ditado.
pub const TOQUE_MAXIMO: f64 = 0.28;
pub const JANELA_DO_SEGUNDO_TOQUE: f64 = 0.4;
/// Outra tecla nesta janela descarta a gravação; depois dela, guarda no histórico sem colar.
pub const JANELA_DE_MODIFICADOR: f64 = 0.7;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Entrada {
    /// A tecla do atalho desceu (`true`) ou subiu (`false`), em segundos monotônicos.
    Gatilho(bool, f64),
    Esc,
    Espaco,
    ColarUltimo,
    OutraTecla(f64),
    /// Passou o tempo; a máquina confere o prazo do segundo toque.
    Tique(f64),
    /// Gravação encerrada fora do atalho (pílula, interface, limite).
    EncerradoPorFora,
    IniciadoEmMaosLivres,
}

/// O que a thread do gancho precisa saber para engolir ou repassar uma tecla.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Visao {
    pub vk: u32,
    pub ativo: bool,
    pub segurando: bool,
}

pub trait Acoes {
    fn comecar(&mut self) -> bool;
    fn terminar(&mut self);
    fn cancelar(&mut self);
    fn maos_livres(&mut self);
    fn colar_ultimo(&mut self);
    fn interromper(&mut self);
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Modo {
    Parado,
    Segurando,
    AguardandoSegundoToque(f64),
    MaosLivres,
}

pub struct Maquina {
    modo: Modo,
    inicio_do_aperto: f64,
    ignorar_ate_soltar: bool,
}

impl Default for Maquina {
    fn default() -> Self {
        Self {
            modo: Modo::Parado,
            inicio_do_aperto: 0.0,
            ignorar_ate_soltar: false,
        }
    }
}

impl Maquina {
    pub fn ativo(&self) -> bool {
        self.modo != Modo::Parado
    }
    pub fn segurando(&self) -> bool {
        self.modo == Modo::Segurando
    }

    pub fn receber(&mut self, e: Entrada, acoes: &mut impl Acoes) {
        match e {
            Entrada::Gatilho(true, t) => match self.modo {
                Modo::Parado => {
                    if acoes.comecar() {
                        self.modo = Modo::Segurando;
                        self.inicio_do_aperto = t;
                    } else {
                        self.ignorar_ate_soltar = true;
                    }
                }
                Modo::AguardandoSegundoToque(_) => {
                    self.modo = Modo::MaosLivres;
                    self.ignorar_ate_soltar = true;
                    acoes.maos_livres();
                }
                Modo::MaosLivres => {
                    self.modo = Modo::Parado;
                    self.ignorar_ate_soltar = true;
                    acoes.terminar();
                }
                Modo::Segurando => {}
            },
            Entrada::Gatilho(false, t) => {
                if self.ignorar_ate_soltar {
                    self.ignorar_ate_soltar = false;
                    return;
                }
                if self.modo != Modo::Segurando {
                    return;
                }
                if t - self.inicio_do_aperto < TOQUE_MAXIMO {
                    self.modo = Modo::AguardandoSegundoToque(t + JANELA_DO_SEGUNDO_TOQUE);
                } else {
                    self.modo = Modo::Parado;
                    acoes.terminar();
                }
            }
            Entrada::Tique(t) => {
                if let Modo::AguardandoSegundoToque(prazo) = self.modo {
                    if t >= prazo {
                        self.modo = Modo::Parado;
                        acoes.cancelar();
                    }
                }
            }
            Entrada::Espaco => {
                if self.modo == Modo::Segurando {
                    self.modo = Modo::MaosLivres;
                    self.ignorar_ate_soltar = true;
                    acoes.maos_livres();
                }
            }
            Entrada::Esc => {
                if self.modo == Modo::Parado {
                    return;
                }
                self.ignorar_ate_soltar = self.modo == Modo::Segurando;
                self.modo = Modo::Parado;
                acoes.cancelar();
            }
            Entrada::OutraTecla(t) => {
                // Ctrl+C, Shift+letra: a tecla foi usada como modificador, não para ditar. Nada é colado.
                if self.modo != Modo::Segurando {
                    return;
                }
                let cedo = t - self.inicio_do_aperto < JANELA_DE_MODIFICADOR;
                self.modo = Modo::Parado;
                self.ignorar_ate_soltar = true;
                if cedo {
                    acoes.cancelar()
                } else {
                    acoes.interromper()
                }
            }
            Entrada::ColarUltimo => acoes.colar_ultimo(),
            Entrada::EncerradoPorFora => {
                if self.modo == Modo::Segurando {
                    self.ignorar_ate_soltar = true;
                }
                self.modo = Modo::Parado;
            }
            Entrada::IniciadoEmMaosLivres => {
                self.modo = Modo::MaosLivres;
            }
        }
    }
}

/// Liga a máquina a uma thread: recebe entradas do gancho e da casca e mantém a [`Visao`] em dia.
pub struct Atalho {
    envio: mpsc::Sender<Entrada>,
    pub visao: Arc<Mutex<Visao>>,
    pub relogio: Instant,
}

impl Atalho {
    pub fn iniciar<A: Acoes + Send + 'static>(vk: u32, mut acoes: A) -> Self {
        let (envio, recebimento) = mpsc::channel::<Entrada>();
        let visao = Arc::new(Mutex::new(Visao {
            vk,
            ativo: false,
            segurando: false,
        }));
        let relogio = Instant::now();
        let v = visao.clone();
        thread::Builder::new()
            .name("ditase.atalho".into())
            .spawn(move || {
                let mut maquina = Maquina::default();
                loop {
                    let entrada = match recebimento.recv_timeout(Duration::from_millis(50)) {
                        Ok(e) => e,
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            Entrada::Tique(relogio.elapsed().as_secs_f64())
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    };
                    maquina.receber(entrada, &mut acoes);
                    let mut atual = v.lock().unwrap();
                    atual.ativo = maquina.ativo();
                    atual.segurando = maquina.segurando();
                }
            })
            .expect("thread do atalho");
        Self {
            envio,
            visao,
            relogio,
        }
    }

    pub fn enviar(&self, e: Entrada) {
        let _ = self.envio.send(e);
    }
    pub fn agora(&self) -> f64 {
        self.relogio.elapsed().as_secs_f64()
    }
    pub fn trocar_tecla(&self, vk: u32) {
        self.visao.lock().unwrap().vk = vk;
    }
    pub fn emissor(&self) -> mpsc::Sender<Entrada> {
        self.envio.clone()
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[derive(Default)]
    struct Registro {
        log: Vec<&'static str>,
        recusar: bool,
    }
    impl Acoes for Registro {
        fn comecar(&mut self) -> bool {
            self.log.push("comecar");
            !self.recusar
        }
        fn terminar(&mut self) {
            self.log.push("terminar")
        }
        fn cancelar(&mut self) {
            self.log.push("cancelar")
        }
        fn maos_livres(&mut self) {
            self.log.push("maosLivres")
        }
        fn colar_ultimo(&mut self) {
            self.log.push("colarUltimo")
        }
        fn interromper(&mut self) {
            self.log.push("interromper")
        }
    }

    fn rodar(entradas: &[Entrada]) -> Vec<&'static str> {
        let mut m = Maquina::default();
        let mut r = Registro::default();
        for e in entradas {
            m.receber(*e, &mut r);
        }
        r.log
    }

    #[test]
    fn segurar_e_soltar_transcreve() {
        assert_eq!(
            rodar(&[Entrada::Gatilho(true, 0.0), Entrada::Gatilho(false, 1.5)]),
            ["comecar", "terminar"]
        );
    }

    #[test]
    fn toque_unico_cancela_depois_da_janela() {
        let log = rodar(&[
            Entrada::Gatilho(true, 0.0),
            Entrada::Gatilho(false, 0.1),
            Entrada::Tique(0.3),
            Entrada::Tique(0.6),
        ]);
        assert_eq!(log, ["comecar", "cancelar"]);
    }

    #[test]
    fn dois_toques_mãos_livres_e_terceiro_encerra() {
        let log = rodar(&[
            Entrada::Gatilho(true, 0.0),
            Entrada::Gatilho(false, 0.1),
            Entrada::Gatilho(true, 0.3),
            Entrada::Gatilho(false, 0.35),
            Entrada::Gatilho(true, 5.0),
            Entrada::Gatilho(false, 5.1),
        ]);
        assert_eq!(log, ["comecar", "maosLivres", "terminar"]);
    }

    #[test]
    fn espaco_segurando_vira_maos_livres() {
        let log = rodar(&[
            Entrada::Gatilho(true, 0.0),
            Entrada::Espaco,
            Entrada::Gatilho(false, 2.0),
        ]);
        assert_eq!(log, ["comecar", "maosLivres"]);
    }

    #[test]
    fn outra_tecla_cedo_cancela_tarde_interrompe() {
        assert_eq!(
            rodar(&[
                Entrada::Gatilho(true, 0.0),
                Entrada::OutraTecla(0.2),
                Entrada::Gatilho(false, 0.5)
            ]),
            ["comecar", "cancelar"]
        );
        assert_eq!(
            rodar(&[
                Entrada::Gatilho(true, 0.0),
                Entrada::OutraTecla(1.2),
                Entrada::Gatilho(false, 1.5)
            ]),
            ["comecar", "interromper"]
        );
    }

    #[test]
    fn esc_cancela_e_soltar_depois_nao_termina() {
        assert_eq!(
            rodar(&[
                Entrada::Gatilho(true, 0.0),
                Entrada::Esc,
                Entrada::Gatilho(false, 2.0)
            ]),
            ["comecar", "cancelar"]
        );
        assert_eq!(rodar(&[Entrada::Esc]), Vec::<&str>::new());
    }

    #[test]
    fn recusa_de_comecar_ignora_o_soltar() {
        let mut m = Maquina::default();
        let mut r = Registro {
            recusar: true,
            ..Default::default()
        };
        m.receber(Entrada::Gatilho(true, 0.0), &mut r);
        m.receber(Entrada::Gatilho(false, 2.0), &mut r);
        assert_eq!(r.log, ["comecar"]);
        assert!(!m.ativo());
    }
}
