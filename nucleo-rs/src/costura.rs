use crate::motores::{PalavraReconhecida, TAXA};
use std::{collections::HashMap, ops::Range};
pub struct AtividadeDeVoz;
impl AtividadeDeVoz {
    const QUADRO: usize = TAXA as usize / 50;

    fn ativo(amostras: &[f32]) -> bool {
        (amostras.iter().map(|a| a * a).sum::<f32>() / amostras.len() as f32).sqrt() >= 0.006
    }

    pub(crate) fn som_curto(amostras: &[f32], limite: usize) -> bool {
        let mut quadros = amostras
            .chunks(Self::QUADRO)
            .enumerate()
            .filter(|(_, q)| Self::ativo(q));
        let Some((primeiro, quadro)) = quadros.next() else {
            return false;
        };
        let (ultimo, tamanho) = quadros
            .next_back()
            .map_or((primeiro, quadro.len()), |(i, q)| (i, q.len()));
        (ultimo - primeiro) * Self::QUADRO + tamanho <= limite
    }

    pub fn tem(amostras: &[f32]) -> bool {
        let mut seguidos = 0;
        for q in amostras.chunks(Self::QUADRO) {
            if Self::ativo(q) {
                seguidos += 1;
                if seguidos == 3 {
                    return true;
                }
            } else {
                seguidos = 0
            }
        }
        false
    }
}
pub struct CaudaDaFala {
    pub inicio: usize,
    pub fim: usize,
}
impl CaudaDaFala {
    pub const CAPACIDADE: usize = 128000;
    pub fn localizar(audio: &[f32]) -> Option<Self> {
        let (mut fim, mut inicio, mut ativos, mut quietos) = (0, 0, 0, 0);
        let mut pos = audio.len();
        for q in audio.rchunks(320) {
            if q.iter().map(|a| a * a).sum::<f32>() / q.len() as f32 >= 0.008 * 0.008 {
                if fim == 0 {
                    fim = pos
                }
                inicio = pos - q.len();
                ativos += q.len();
                quietos = 0
            } else if fim > 0 {
                quietos += 1;
                if quietos >= 13 {
                    break;
                }
            }
            pos -= q.len()
        }
        (ativos >= 1920).then_some(Self { inicio, fim })
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FonteDeAudio {
    Mic,
    Mac,
    Arquivo,
}
impl FonteDeAudio {
    pub fn nome(self) -> &'static str {
        match self {
            Self::Mic => "mic",
            Self::Mac => "mac",
            Self::Arquivo => "arquivo",
        }
    }
    pub fn voz_ao_vivo(self) -> &'static str {
        match self {
            Self::Mic => "voce",
            Self::Mac => "chamada",
            Self::Arquivo => "desconhecida",
        }
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct ResultadoCostura {
    pub confirmadas: Vec<PalavraReconhecida>,
    pub novo_confirmado: usize,
    pub pendente: Option<Range<usize>>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct TrechoNovo {
    pub id: String,
    pub fonte: FonteDeAudio,
    pub inicio: f64,
    pub fim: f64,
    pub texto: String,
    pub voz: String,
}
pub struct Costura;
impl Costura {
    pub fn decidir(
        palavras: &[PalavraReconhecida],
        confirmado_ate: usize,
        limite: usize,
        fechamento: bool,
        atividade: impl Fn(Range<usize>) -> bool,
    ) -> ResultadoCostura {
        let taxa = f64::from(TAXA);
        let c = confirmado_ate as f64 / taxa;
        let l = limite as f64 / taxa;
        let centro = |p: &PalavraReconhecida| (p.inicio + p.fim) / 2.;
        let mut candidatas: Vec<_> = palavras.iter().filter(|p| centro(p) > c).cloned().collect();
        candidatas.sort_by(|a, b| a.inicio.total_cmp(&b.inicio));
        let mut r = ResultadoCostura {
            confirmadas: vec![],
            novo_confirmado: confirmado_ate,
            pendente: None,
        };
        if limite <= confirmado_ate {
            return r;
        }
        if fechamento || candidatas.is_empty() {
            r.confirmadas = candidatas.into_iter().filter(|p| centro(p) <= l).collect();
            let voz = r.confirmadas.is_empty() && atividade(confirmado_ate..limite);
            if fechamento || !voz || l - c > 10. {
                r.novo_confirmado = limite;
                if voz {
                    r.pendente = Some(confirmado_ate..limite)
                }
            }
            return r;
        }
        let mut corte = l;
        while corte > c {
            match candidatas
                .iter()
                .find(|p| p.inicio - 0.1 < corte && corte < p.fim + 0.1)
            {
                Some(p) => corte = p.inicio - 0.1,
                None => break,
            }
        }
        if corte <= c && l - c > 10. {
            if let Some(p) = candidatas.iter().rfind(|p| p.fim <= l - 1. && p.fim > c) {
                corte = p.fim
            }
        }
        if corte > c {
            r.novo_confirmado = ((corte * taxa).round() as usize).clamp(confirmado_ate + 1, limite);
            r.confirmadas = candidatas
                .into_iter()
                .filter(|p| centro(p) < r.novo_confirmado as f64 / taxa)
                .collect()
        }
        r
    }
    pub fn trechos(
        palavras: &[PalavraReconhecida],
        documento: &str,
        fonte: FonteDeAudio,
        geracao: u64,
        prefixo: Option<&str>,
    ) -> Vec<TrechoNovo> {
        let mut palavras = palavras.to_vec();
        palavras.sort_by(|a, b| a.inicio.total_cmp(&b.inicio));
        let mut saida = Vec::<TrechoNovo>::new();
        let mut usados = HashMap::new();
        let prefixo = prefixo
            .map(str::to_owned)
            .unwrap_or_else(|| format!("{documento}|{}|{geracao}", fonte.nome()));
        for p in palavras {
            if let Some(u) = saida.last_mut() {
                if p.inicio - u.fim < 1.2
                    && p.fim - u.inicio <= 15.
                    && !u.texto.ends_with(['.', '?', '!'])
                {
                    u.texto.push(' ');
                    u.texto.push_str(&p.texto);
                    u.fim = u.fim.max(p.fim);
                    continue;
                }
            }
            let amostra = (p.inicio * f64::from(TAXA)).round() as i64;
            let n = usados.entry(amostra).or_insert(0);
            let mut id = format!("{prefixo}|{amostra}");
            if *n > 0 {
                id += &format!("+{n}")
            }
            *n += 1;
            saida.push(TrechoNovo {
                id,
                inicio: p.inicio,
                fim: p.fim,
                texto: p.texto,
                fonte,
                voz: fonte.voz_ao_vivo().into(),
            })
        }
        saida
    }
}
