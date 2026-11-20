use crate::Palavras;
use std::sync::Arc;
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Termo {
    pub id: i64,
    pub termo: String,
    pub ouvido_como: Vec<String>,
    pub usos: i64,
    pub origem: String,
    pub criado_em: f64,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Troca {
    pub de: String,
    pub para: String,
    pub termo_id: i64,
}
pub type PalavraComum = Arc<dyn Fn(&str) -> bool + Send + Sync>;
#[derive(Clone, Default)]
pub struct AplicadorDicionario {
    termos: Vec<Termo>,
    entradas: Vec<(String, usize)>,
    sons: Vec<(String, usize)>,
    pub vocabulario: Vec<String>,
    comum: Option<PalavraComum>,
}
impl AplicadorDicionario {
    pub fn new(termos: Vec<Termo>, comum: Option<PalavraComum>) -> Self {
        let mut entradas = vec![];
        let mut sons = vec![];
        for (i, t) in termos.iter().enumerate() {
            for v in std::iter::once(&t.termo).chain(&t.ouvido_como) {
                let chave = Palavras::normalizadas(v).concat();
                if !chave.is_empty() {
                    entradas.push((chave, i))
                }
            }
            let som = Self::chave_de_som(&t.termo);
            if som.chars().count() >= 5 {
                sons.push((som, i))
            }
        }
        let vocabulario = termos.iter().map(|t| t.termo.clone()).collect();
        Self {
            termos,
            entradas,
            sons,
            comum,
            vocabulario,
        }
    }
    pub fn chave_de_som(t: &str) -> String {
        let mut s = Palavras::normalizar(&t.to_lowercase().replace('ç', "s"));
        for (de, para) in [
            ("ph", "f"),
            ("ch", "x"),
            ("sh", "x"),
            ("lh", "li"),
            ("nh", "ni"),
            ("ck", "k"),
            ("th", "t"),
            ("que", "ke"),
            ("qui", "ki"),
            ("qu", "ku"),
            ("gue", "ge"),
            ("gui", "gi"),
            ("sce", "se"),
            ("sci", "si"),
            ("xce", "se"),
            ("xci", "si"),
            ("ce", "se"),
            ("ci", "si"),
            ("c", "k"),
            ("q", "k"),
            ("y", "i"),
            ("w", "u"),
            ("z", "s"),
            ("h", ""),
            ("ou", "o"),
            ("ei", "e"),
        ] {
            s = s.replace(de, para)
        }
        let mut chars: Vec<_> = s.chars().collect();
        chars.dedup();
        if let Some(c) = chars.last_mut() {
            *c = match c {
                'e' => 'i',
                'o' => 'u',
                'm' => 'n',
                _ => *c,
            }
        }
        chars.into_iter().collect()
    }
    pub fn semelhanca(a: &str, b: &str) -> f64 {
        let a: Vec<_> = a.chars().collect();
        let b: Vec<_> = b.chars().collect();
        if a.is_empty() || b.is_empty() {
            return 0.;
        }
        let mut ant: Vec<_> = (0..=b.len()).collect();
        for (i, x) in a.iter().enumerate() {
            let mut linha = vec![i + 1];
            for (j, y) in b.iter().enumerate() {
                linha.push(
                    (linha[j] + 1)
                        .min(ant[j + 1] + 1)
                        .min(ant[j] + usize::from(x != y)),
                )
            }
            ant = linha
        }
        1. - ant[b.len()] as f64 / a.len().max(b.len()) as f64
    }
    fn por_som(&self, partes: &[regex::Match<'_>]) -> Option<usize> {
        let curta = |p: &regex::Match<'_>| {
            p.as_str().chars().count() < if partes.len() == 1 { 6 } else { 3 }
        };
        if partes.iter().any(|p| {
            self.comum
                .as_ref()
                .map_or_else(|| curta(p), |f| f(p.as_str()))
        }) {
            return None;
        }
        let som = Self::chave_de_som(&partes.iter().map(|p| p.as_str()).collect::<String>());
        if som.len() < 4 {
            return None;
        }
        let mut melhor = 0.;
        let mut id = None;
        for (chave, i) in &self.sons {
            if chave.len().abs_diff(som.len()) <= 2 {
                let v = Self::semelhanca(&som, chave);
                if v >= 0.8 && v > melhor {
                    melhor = v;
                    id = Some(*i)
                }
            }
        }
        id
    }
    pub fn aplicar(&self, t: &str) -> (String, Vec<Troca>) {
        let p: Vec<_> = Palavras::ocorrencias(t).collect();
        let n = Palavras::normalizadas(t);
        let mut i = 0;
        let mut cursor = 0;
        let mut saida = String::new();
        let mut trocas = vec![];
        while i < p.len() {
            let mut usados = 1;
            for janela in (1..=4.min(p.len() - i)).rev() {
                let fim = p[i + janela - 1].end();
                if janela > 1
                    && t[p[i].end()..p[i + janela - 1].start()]
                        .contains(['.', ',', ';', ':', '!', '?', '…', '\n', '(', ')', '"'])
                {
                    continue;
                }
                let chave = n[i..i + janela].concat();
                let id = self
                    .entradas
                    .iter()
                    .find(|(k, _)| *k == chave)
                    .map(|e| e.1)
                    .or_else(|| {
                        if janela <= 2 {
                            self.por_som(&p[i..i + janela])
                        } else {
                            None
                        }
                    });
                if let Some(id) = id {
                    let termo = &self.termos[id];
                    let de = &t[p[i].start()..fim];
                    if de != termo.termo {
                        saida.push_str(&t[cursor..p[i].start()]);
                        saida.push_str(&termo.termo);
                        cursor = fim;
                        trocas.push(Troca {
                            de: de.to_owned(),
                            para: termo.termo.clone(),
                            termo_id: termo.id,
                        })
                    }
                    usados = janela;
                    break;
                }
            }
            i += usados
        }
        saida.push_str(&t[cursor..]);
        (saida, trocas)
    }
}
