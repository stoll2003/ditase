use std::ops::Range;
#[derive(Clone, Debug, PartialEq)]
pub struct LinhaDoTempo {
    pub inicio: f64,
    pausas: Vec<(f64, Option<f64>)>,
}
impl LinhaDoTempo {
    pub fn new(inicio: f64) -> Self {
        Self {
            inicio,
            pausas: vec![],
        }
    }
    pub fn pausada(&self) -> bool {
        matches!(self.pausas.last(), Some((_, None)))
    }
    pub fn pausar(&mut self, t: f64) {
        if !self.pausada() {
            self.pausas.push((t, None))
        }
    }
    pub fn retomar(&mut self, t: f64) {
        if let Some((de, ate @ None)) = self.pausas.last_mut() {
            *ate = Some(de.max(t))
        }
    }
    pub fn indice(&self, t: f64) -> Option<i64> {
        if t < self.inicio {
            return None;
        }
        let mut parado = 0.;
        for &(de, ate) in &self.pausas {
            if t < de {
                break;
            }
            let fim = ate?;
            if fim > t {
                return None;
            }
            parado += fim - de
        }
        Some(((t - self.inicio - parado) * 16000.).round() as i64)
    }
    pub fn partes_ativas(&self, t: f64, n: usize) -> Vec<Parte> {
        let mut limites = vec![0, n];
        for marco in std::iter::once(self.inicio).chain(
            self.pausas
                .iter()
                .flat_map(|(a, b)| std::iter::once(*a).chain(*b)),
        ) {
            let k = ((marco - t) * 16000.).round() as i64;
            if k > 0 && (k as usize) < n {
                limites.push(k as usize)
            }
        }
        limites.sort_unstable();
        limites.dedup();
        let mut partes = Vec::<Parte>::new();
        for corte in limites.windows(2) {
            let origem = corte[0];
            let quantidade = corte[1] - origem;
            let metade = quantidade as f64 / 2.;
            if let Some(m) = self.indice(t + (origem as f64 + metade) / 16000.) {
                let indice = m - metade.round() as i64;
                if let Some(p) = partes.last_mut() {
                    if p.origem + p.quantidade == origem && p.indice + p.quantidade as i64 == indice
                    {
                        p.quantidade += quantidade;
                        continue;
                    }
                }
                partes.push(Parte {
                    origem,
                    quantidade,
                    indice,
                })
            }
        }
        partes
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parte {
    pub origem: usize,
    pub quantidade: usize,
    pub indice: i64,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Evento {
    Amostras(Parte),
    Lacuna(Range<i64>),
}
pub struct CursorDeFonte {
    pub proximo: i64,
}
impl CursorDeFonte {
    pub fn new(abertura: i64) -> Self {
        Self { proximo: abertura }
    }
    pub fn receber(&mut self, t: f64, n: usize, linha: &LinhaDoTempo) -> Vec<Evento> {
        let mut eventos = vec![];
        for mut p in linha.partes_ativas(t, n) {
            let dif = p.indice - self.proximo;
            if dif > 960 {
                eventos.push(Evento::Lacuna(self.proximo..p.indice));
                self.proximo = p.indice
            } else if dif < -960 {
                let repetidas = p.quantidade.min((-dif) as usize);
                p.quantidade -= repetidas;
                p.origem += repetidas
            }
            if p.quantidade > 0 {
                p.indice = self.proximo;
                self.proximo += p.quantidade as i64;
                eventos.push(Evento::Amostras(p))
            }
        }
        eventos
    }
}
