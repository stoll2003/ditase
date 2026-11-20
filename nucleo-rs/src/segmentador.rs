#[derive(Debug, Clone)]
pub struct Segmento {
    pub inicio: usize,
    pub amostras: Vec<f32>,
    pub pausa_antes: f64,
    pub cortado_no_meio: bool,
}
impl Segmento {
    pub fn duracao(&self) -> f64 {
        self.amostras.len() as f64 / 16000.0
    }
}

#[derive(Clone, Debug)]
pub struct Config {
    pub taxa: usize,
    pub pausa_corte: f64,
    pub pausa_longa: f64,
    pub minimo_para_corte: f64,
    pub maximo: f64,
    pub margem: f64,
    pub fala_minima: f64,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            taxa: 16000,
            pausa_corte: 0.55,
            pausa_longa: 1.1,
            minimo_para_corte: 5.,
            maximo: 13.5,
            margem: 0.2,
            fala_minima: 0.16,
        }
    }
}

#[derive(Default)]
pub struct Segmentador {
    pub config: Config,
    resto: Vec<f32>,
    atual: Vec<f32>,
    niveis: Vec<f32>,
    pre: std::collections::VecDeque<f32>,
    em_fala: bool,
    falados: usize,
    quietos: usize,
    parado: usize,
    posicao: usize,
    inicio: usize,
    pausa: f64,
    pico: f32,
    pub nivel: f32,
    pub fala_total: f64,
}
impl Segmentador {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            ..Default::default()
        }
    }
    pub fn silencio_corrido(&self) -> f64 {
        (if self.em_fala {
            self.quietos
        } else {
            self.parado
        }) as f64
            * 0.02
    }
    pub fn alimentar(&mut self, novas: &[f32]) -> Vec<Segmento> {
        let mut dados = std::mem::take(&mut self.resto);
        dados.extend_from_slice(novas);
        let (quadros, resto) = dados.as_chunks::<320>();
        let mut prontos = Vec::new();
        for q in quadros {
            if let Some(s) = self.processar(q) {
                prontos.push(s)
            }
            self.posicao += 320;
        }
        self.resto.extend_from_slice(resto);
        prontos
    }
    pub fn finalizar(&mut self) -> Option<Segmento> {
        self.posicao += self.resto.len();
        if self.em_fala {
            self.atual.append(&mut self.resto)
        }
        self.resto.clear();
        let s = (self.em_fala && self.falados as f64 * 0.02 >= self.config.fala_minima)
            .then(|| self.fechar(self.atual.len(), false));
        self.reiniciar();
        s
    }
    fn fechar(&mut self, fim: usize, cortado: bool) -> Segmento {
        let sobra = self.atual.split_off(fim);
        let amostras = std::mem::replace(&mut self.atual, sobra);
        Segmento {
            inicio: self.inicio,
            amostras,
            pausa_antes: self.pausa,
            cortado_no_meio: cortado,
        }
    }
    fn reiniciar(&mut self) {
        self.em_fala = false;
        self.atual.clear();
        self.niveis.clear();
        self.pre.clear();
        self.falados = 0;
        self.quietos = 0;
        self.parado = 0;
    }
    fn processar(&mut self, q: &[f32]) -> Option<Segmento> {
        let rms = (q.iter().map(|a| a * a).sum::<f32>() / 320.).sqrt();
        self.pico = (self.pico * 0.9995).max(rms);
        self.nivel = (rms * 12.).min(1.);
        let limiar = (self.pico * 0.1).clamp(0.0018, 0.008);
        let fala = rms >= limiar;
        let margem = (self.config.margem * self.config.taxa as f64) as usize;
        if !self.em_fala {
            if !fala {
                self.parado += 1;
                self.pre.extend(q);
                self.pre.drain(..self.pre.len().saturating_sub(margem));
                return None;
            }
            self.em_fala = true;
            self.inicio = self.posicao - self.pre.len();
            self.pausa = self.parado as f64 * 0.02;
            self.niveis.resize(self.pre.len() / 320, 0.);
            self.atual.extend(self.pre.drain(..));
        }
        self.atual.extend_from_slice(q);
        self.niveis.push(rms);
        if fala {
            self.falados += 1;
            self.quietos = 0;
            self.fala_total += 0.02
        } else {
            self.quietos += 1
        }
        let quieto = self.quietos as f64 * 0.02;
        let duracao = self.atual.len() as f64 / self.config.taxa as f64;
        if quieto >= self.config.pausa_longa
            || quieto >= self.config.pausa_corte && duracao >= self.config.minimo_para_corte
        {
            let fim = self.atual.len() - (self.quietos * 320).saturating_sub(margem);
            let parado = self.quietos;
            let s = (self.falados as f64 * 0.02 >= self.config.fala_minima)
                .then(|| self.fechar(fim, false));
            self.reiniciar();
            self.parado = parado;
            return s;
        }
        if duracao >= self.config.maximo {
            let total = self.niveis.len();
            let inicio = 5.max(total.saturating_sub(200));
            let fim = total.saturating_sub(10);
            let corte = (inicio..fim)
                .min_by(|&a, &b| {
                    let soma = |i: usize| self.niveis[i - 2..=i + 2].iter().sum::<f32>();
                    soma(a).total_cmp(&soma(b))
                })
                .unwrap_or(total.saturating_sub(1))
                .max(1);
            let s = self.fechar(corte * 320, true);
            self.niveis.drain(..corte);
            self.inicio += corte * 320;
            self.pausa = 0.;
            self.quietos = 0;
            self.parado = 0;
            self.falados = self.niveis.iter().filter(|v| **v >= limiar).count();
            return Some(s);
        }
        None
    }
}
