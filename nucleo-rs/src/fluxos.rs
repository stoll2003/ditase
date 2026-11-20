use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread,
    time::{Duration, Instant},
};
#[derive(Default)]
pub struct SaidaComPrazo {
    em_curso: Arc<AtomicBool>,
}
impl SaidaComPrazo {
    pub fn em_curso(&self) -> bool {
        self.em_curso.load(Ordering::Acquire)
    }
    pub fn iniciar(
        &self,
        prazo: Duration,
        trabalho: impl FnOnce() + Send + 'static,
        responder: impl FnOnce() + Send + 'static,
    ) -> bool {
        if self.em_curso.swap(true, Ordering::AcqRel) {
            return false;
        }
        let inicio = Instant::now();
        let (tx, rx) = mpsc::channel();
        let curso = self.em_curso.clone();
        thread::spawn(move || {
            let _ = rx.recv_timeout(prazo.saturating_sub(inicio.elapsed()));
            curso.store(false, Ordering::Release);
            responder()
        });
        thread::spawn(move || {
            trabalho();
            let _ = tx.send(());
        });
        true
    }
}
pub struct DistribuidorDoMicrofone<F> {
    para_reuniao: F,
    derivacao: Option<mpsc::SyncSender<Vec<f32>>>,
    pub estouros_da_derivacao: usize,
}
impl<F: FnMut(f64, &[f32])> DistribuidorDoMicrofone<F> {
    pub fn new(para_reuniao: F) -> Self {
        Self {
            para_reuniao,
            derivacao: None,
            estouros_da_derivacao: 0,
        }
    }
    pub fn abrir_derivacao(&mut self, limite: usize) -> mpsc::Receiver<Vec<f32>> {
        let (tx, rx) = mpsc::sync_channel(limite);
        self.derivacao = Some(tx);
        rx
    }
    pub fn fechar_derivacao(&mut self) {
        self.derivacao = None
    }
    pub fn derivacao_aberta(&self) -> bool {
        self.derivacao.is_some()
    }
    pub fn entregar(&mut self, tempo: f64, amostras: &[f32]) {
        (self.para_reuniao)(tempo, amostras);
        if let Some(tx) = &self.derivacao {
            match tx.try_send(amostras.to_vec()) {
                Ok(()) => {}
                Err(mpsc::TrySendError::Full(_)) => {
                    self.estouros_da_derivacao += 1;
                    self.derivacao = None
                }
                Err(mpsc::TrySendError::Disconnected(_)) => self.derivacao = None,
            }
        }
    }
}
