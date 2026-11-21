//! Emenda do ditado derivado da reunião com o microfone próprio (N-1).
//!
//! Quando a reunião acaba no meio de um ditado derivado, o microfone próprio abre com a captura da
//! reunião ainda aberta: as duas fontes rodam juntas por um instante. Cada fonte tem o seu
//! [`RelogioDeFonte`] (contagem de amostras desde o primeiro bloco, no mesmo relógio de parede), e
//! a emenda corta pelo tempo:
//!
//! - enquanto a derivação vive, ela é a fonte do ditado; os blocos do microfone próprio esperam;
//! - quando a derivação fecha, o corte é o fim do último bloco dela; do microfone próprio entra só
//!   o que vem depois do corte (o que cai antes é a mesma fala, já entregue pela derivação).
//!
//! Assim não há lacuna enquanto o microfone próprio abre nem fala duplicada na sobreposição. O erro
//! que sobra é o de estimar o início de cada fonte pela chegada do primeiro bloco (alguns ms).
use std::{
    sync::{Arc, Mutex},
    time::Instant,
};

use crate::{audio::TAXA, controlador::AoBlocoDeAudio, lancamento::RelogioDeFonte};

/// Destino dos blocos do ditado; `None` depois que o ditado termina.
pub type Destino = Arc<Mutex<Option<AoBlocoDeAudio>>>;

const MARGEM: f64 = 60.0;

pub struct Emenda {
    base: Instant,
    destino: Destino,
    derivado: RelogioDeFonte,
    proprio: RelogioDeFonte,
    /// Fim do último bloco entregue pela derivação (segundos desde `base`).
    fim_derivado: Option<f64>,
    /// Definido quando a derivação fecha: daqui em diante só o microfone próprio, a partir deste tempo.
    corte: Option<f64>,
    /// Blocos do microfone próprio que chegaram com a derivação ainda viva: (início, PCM, nível).
    pendentes: Vec<(f64, Vec<f32>, f32)>,
    proprio_chegou: bool,
}

impl Emenda {
    pub fn new(destino: Destino, base: Instant) -> Self {
        Self {
            base,
            destino,
            derivado: RelogioDeFonte::default(),
            proprio: RelogioDeFonte::default(),
            fim_derivado: None,
            corte: None,
            pendentes: Vec::new(),
            proprio_chegou: false,
        }
    }

    /// Segundos desde `base` com uma margem: o primeiro bloco chega logo depois de criada a emenda,
    /// e sem a margem o [`RelogioDeFonte`] prenderia o início dele em zero (atrasando aquela fonte).
    fn segundos(&self, agora: Instant) -> f64 {
        MARGEM + agora.saturating_duration_since(self.base).as_secs_f64()
    }

    /// Bloco da derivação, recebido em `agora` (fim do bloco).
    pub fn derivado(&mut self, bloco: Vec<f32>, nivel: f32, agora: Instant) {
        if self.corte.is_some() || bloco.is_empty() {
            return;
        }
        let n = bloco.len();
        let t = self.derivado.tempo(self.segundos(agora), n);
        self.fim_derivado = Some(t + n as f64 / TAXA as f64);
        self.entregar(bloco, nivel);
    }

    /// Bloco do microfone próprio, recebido em `agora` (fim do bloco).
    pub fn proprio(&mut self, bloco: Vec<f32>, nivel: f32, agora: Instant) {
        if bloco.is_empty() {
            return;
        }
        self.proprio_chegou = true;
        let t = self.proprio.tempo(self.segundos(agora), bloco.len());
        match self.corte {
            None => self.pendentes.push((t, bloco, nivel)),
            Some(c) => self.entregar_depois_de(c, t, bloco, nivel),
        }
    }

    /// A derivação fechou (a reunião soltou o microfone ou o ditado trocou de fonte). Idempotente.
    pub fn fechar_derivado(&mut self) {
        if self.corte.is_some() {
            return;
        }
        let c = self.fim_derivado.unwrap_or(f64::NEG_INFINITY);
        self.corte = Some(c);
        for (t, bloco, nivel) in std::mem::take(&mut self.pendentes) {
            self.entregar_depois_de(c, t, bloco, nivel);
        }
    }

    /// Já chegou algum bloco do microfone próprio.
    pub fn proprio_chegou(&self) -> bool {
        self.proprio_chegou
    }

    fn entregar_depois_de(&mut self, corte: f64, inicio: f64, mut bloco: Vec<f32>, nivel: f32) {
        let antes = ((corte - inicio) * TAXA as f64).round();
        if antes >= bloco.len() as f64 {
            return;
        }
        if antes > 0.0 {
            bloco.drain(..antes as usize);
        }
        self.entregar(bloco, nivel);
    }

    fn entregar(&self, bloco: Vec<f32>, nivel: f32) {
        if let Some(f) = self.destino.lock().unwrap().as_mut() {
            f(bloco, nivel);
        }
    }
}

/// Guardado dentro do callback da derivação: quando a derivação solta o callback (reunião acabou
/// ou o ditado trocou de fonte), fecha a derivação na emenda.
pub struct FimDaDerivacao(pub Arc<Mutex<Emenda>>);

impl Drop for FimDaDerivacao {
    fn drop(&mut self) {
        self.0.lock().unwrap().fechar_derivado();
    }
}

#[cfg(test)]
mod testes {
    use super::*;
    use std::time::Duration;

    /// A fala real: amostra `k` vale `k`. Assim dá para ver lacuna e repetição na saída.
    fn fala(de: u64, ate: u64) -> Vec<f32> {
        (de..ate).map(|k| k as f32).collect()
    }

    struct Cena {
        base: Instant,
        saida: Arc<Mutex<Vec<f32>>>,
        emenda: Emenda,
    }

    fn cena() -> Cena {
        let base = Instant::now();
        let saida = Arc::new(Mutex::new(Vec::new()));
        let s = saida.clone();
        let destino: Destino = Arc::new(Mutex::new(Some(Box::new(move |b: Vec<f32>, _n| {
            s.lock().unwrap().extend(b)
        }))));
        Cena {
            base,
            saida,
            emenda: Emenda::new(destino, base),
        }
    }

    impl Cena {
        /// Instante em que a amostra `k` termina de chegar.
        fn em(&self, k: u64) -> Instant {
            self.base + Duration::from_secs_f64(k as f64 / TAXA as f64)
        }
        fn derivado(&mut self, de: u64, ate: u64) {
            let agora = self.em(ate);
            self.emenda.derivado(fala(de, ate), 0.3, agora);
        }
        fn proprio(&mut self, de: u64, ate: u64) {
            let agora = self.em(ate);
            self.emenda.proprio(fala(de, ate), 0.3, agora);
        }
        fn conferir_continua(&self, de: u64, ate: u64) {
            let saida = self.saida.lock().unwrap();
            assert_eq!(
                saida.len() as u64,
                ate - de,
                "tamanho errado (lacuna ou repetição)"
            );
            for (i, x) in saida.iter().enumerate() {
                assert_eq!(*x as u64, de + i as u64, "quebra na amostra {i}");
            }
        }
    }

    /// Microfone próprio abre com a derivação viva, com blocos desalinhados dos da derivação:
    /// a saída é a fala inteira, sem lacuna nem repetição.
    #[test]
    fn sobreposicao_emenda_sem_perda_nem_duplicacao() {
        let mut c = cena();
        for i in 0..5 {
            c.derivado(i * 1600, (i + 1) * 1600);
        }
        // O microfone próprio começa a captar na amostra 7000, com blocos de 480 amostras.
        let mut k = 7000;
        while k + 480 <= 9600 {
            c.proprio(k, k + 480);
            k += 480;
        }
        c.derivado(8000, 9600);
        c.emenda.fechar_derivado();
        // Derivação depois do corte não entra mais.
        c.derivado(9600, 11200);
        while k + 480 <= 14_000 {
            c.proprio(k, k + 480);
            k += 480;
        }
        c.conferir_continua(0, k);
    }

    /// Fallback: o microfone próprio só abriu depois que a reunião soltou (há lacuna real, que o
    /// ditado avisa), mas nada se repete.
    #[test]
    fn microfone_aberto_depois_do_corte_nao_repete() {
        let mut c = cena();
        c.derivado(0, 1600);
        c.derivado(1600, 3200);
        c.emenda.fechar_derivado();
        c.proprio(4000, 4800);
        c.proprio(4800, 5600);
        let saida = c.saida.lock().unwrap();
        assert_eq!(saida.len(), 3200 + 1600);
        assert_eq!(saida[3199] as u64, 3199);
        assert_eq!(saida[3200] as u64, 4000);
    }

    /// Derivação sem nenhum bloco (ditado acabou de começar): tudo do microfone próprio entra.
    #[test]
    fn sem_bloco_derivado_entrega_todo_o_microfone_proprio() {
        let mut c = cena();
        c.proprio(0, 800);
        c.emenda.fechar_derivado();
        c.proprio(800, 1600);
        c.conferir_continua(0, 1600);
    }

    /// O ditado terminou (destino vazio): nada é entregue e nada quebra.
    #[test]
    fn ditado_terminado_nao_recebe_nada() {
        let mut c = cena();
        c.derivado(0, 1600);
        c.emenda.destino.lock().unwrap().take();
        c.proprio(1000, 2000);
        c.emenda.fechar_derivado();
        assert_eq!(c.saida.lock().unwrap().len(), 1600);
    }
}
