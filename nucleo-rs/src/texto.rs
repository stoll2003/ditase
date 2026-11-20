use regex::Regex;
use std::ops::Range;
use std::sync::LazyLock;
use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};
static PALAVRA: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[\pL\pN][\pL\pM\pN]*(?:[-'’][\pL\pN][\pL\pM\pN]*)*").unwrap());
pub struct Palavras;
impl Palavras {
    pub fn ocorrencias(t: &str) -> impl Iterator<Item = regex::Match<'_>> {
        PALAVRA.find_iter(t)
    }
    pub fn normalizar(t: &str) -> String {
        t.nfd()
            .filter(|&c| !is_combining_mark(c) && c.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect()
    }
    pub fn normalizadas(t: &str) -> Vec<String> {
        Self::ocorrencias(t)
            .map(|p| Self::normalizar(p.as_str()))
            .collect()
    }
    pub fn contar(t: &str) -> usize {
        Self::ocorrencias(t).count()
    }
    pub fn cauda(t: &str, n: usize) -> &str {
        let p: Vec<_> = Self::ocorrencias(t).collect();
        if n == 0 || n >= p.len() {
            t
        } else {
            &t[p[p.len() - n].start()..]
        }
    }
}
fn regras(padroes: &[(&'static str, &'static str)]) -> Vec<(Regex, &'static str)> {
    padroes
        .iter()
        .map(|(p, r)| (Regex::new(p).unwrap(), *r))
        .collect()
}
static COMANDOS: LazyLock<Vec<(Regex, &str)>> = LazyLock::new(|| {
    regras(&[
        (
            r"(?i)(^|[.,;:!?…]\s*|\n\s*)nov[oa]\s+par[aá]grafo\b[.,;:!?…]?\s*",
            "${1}\n\n",
        ),
        (
            r"(?i)(^|[.,;:!?…]\s*|\n\s*)nova\s+linha\b[.,;:!?…]?\s*",
            "${1}\n",
        ),
        (r"[,;:]\s*\n", ".\n"),
    ])
});
static ESPACOS: LazyLock<Vec<(Regex, &str)>> = LazyLock::new(|| {
    regras(&[
        (r"[ \t]+", " "),
        (r" +([,.;:!?…])", "$1"),
        (r",{2,}", ","),
        (r";{2,}", ";"),
        (r":{2,}", ":"),
        (r",\s*([.!?])", "$1"),
        (r"(?m)^[ ,;:.]+", ""),
        (r" *\n *", "\n"),
        (r"\n{3,}", "\n\n"),
    ])
});
pub struct RegrasFala;
impl RegrasFala {
    pub fn arrumar_espacos(t: &str) -> String {
        let mut t = t.to_owned();
        for (p, r) in ESPACOS.iter() {
            t = p.replace_all(&t, *r).into_owned()
        }
        let (mut inicio, mut apos) = (true, false);
        let mut saida = String::new();
        for c in t.trim().chars() {
            if inicio && c.is_alphabetic() {
                saida.extend(c.to_uppercase());
                inicio = false;
                apos = false;
                continue;
            }
            saida.push(c);
            match c {
                '\n' => inicio = true,
                '.' | '!' | '?' | '…' => apos = true,
                '"' | '“' | '(' | '\'' | '”' | ')' => {}
                c if c.is_whitespace() => {
                    if apos {
                        inicio = true
                    }
                }
                _ => {
                    inicio = false;
                    apos = false
                }
            }
        }
        saida
    }
    pub fn filtrar(bruto: &str) -> String {
        let mut t = bruto.replace('\u{a0}', " ");
        for (p, r) in COMANDOS.iter() {
            t = p.replace_all(&t, *r).into_owned()
        }
        let mut s = String::new();
        let mut cursor = 0;
        for (i, p) in Palavras::ocorrencias(&t).enumerate() {
            let original = p.as_str().to_lowercase();
            let k = Palavras::normalizar(&original);
            let resto = &t[p.end()..];
            let pausa = resto.starts_with([',', '.', '…']);
            let certa = matches!(
                k.as_str(),
                "han"
                    | "ahn"
                    | "ahm"
                    | "hum"
                    | "hm"
                    | "hmm"
                    | "uhm"
                    | "uh"
                    | "ehh"
                    | "ee"
                    | "eee"
                    | "aa"
                    | "aaa"
            );
            if original != "há"
                && (certa
                    || original == "hã"
                    || pausa
                        && (k == "ha" || k == "eh" || original == "um-hum" || k == "ah" && i == 0)
                    || original == "é" && resto.starts_with("..."))
            {
                s.push_str(&t[cursor..p.start()]);
                let apos = resto.trim_start_matches([',', '.', '…']);
                let pont = &resto[..resto.len() - apos.len()];
                if pont.contains('.')
                    && !s.trim_end().is_empty()
                    && !s
                        .trim_end_matches([' ', '\t'])
                        .ends_with(['.', '!', '?', '…', ',', ';', ':', '\n'])
                {
                    s.truncate(s.trim_end().len());
                    s.push_str(". ")
                }
                cursor = t.len() - apos.len()
            }
        }
        s.push_str(&t[cursor..]);
        let palavras: Vec<_> = Palavras::ocorrencias(&s).collect();
        let mut remover = Vec::<Range<usize>>::new();
        for w in palavras.windows(2) {
            let (a, b) = (w[0], w[1]);
            let entre = &s[a.end()..b.start()];
            if !entre.chars().all(|c| c == ' ' || c == ',') {
                continue;
            }
            let ka = a.as_str().to_lowercase();
            let kb = b.as_str().to_lowercase();
            let liga="a o as os um uma de do da dos das em no na nos nas e que se com por para pra pro eu ele ela isso esse essa este esta ao aos ou mas porque como quando meu minha".split_whitespace().any(|p|p==ka);
            if ka == kb && liga
                || ka.chars().count() <= 2
                    && kb.starts_with(&ka)
                    && kb.len() > ka.len()
                    && entre.contains(',')
                    && !liga
            {
                remover.push(a.start()..b.start())
            }
        }
        for r in remover.into_iter().rev() {
            s.replace_range(r, "")
        }
        Self::arrumar_espacos(&s)
    }
}
#[derive(Debug, PartialEq, Eq)]
pub enum Recusa {
    Vazio,
    Acrescentou,
    CortouDemais,
    Respondeu,
}
pub struct Guarda;
impl Guarda {
    pub fn avaliar(original: &str, limpo: &str) -> Option<Recusa> {
        let antes = Palavras::normalizadas(original);
        let depois = Palavras::normalizadas(limpo);
        if antes.is_empty() {
            return (!depois.is_empty()).then_some(Recusa::Acrescentou);
        }
        if depois.is_empty() {
            return (antes.len() > 3).then_some(Recusa::Vazio);
        }
        let mut disponiveis = std::collections::HashMap::new();
        for p in &antes {
            *disponiveis.entry(p).or_insert(0) += 1
        }
        let mut novas = 0;
        for p in &depois {
            if let Some(n) = disponiveis.get_mut(p) {
                if *n > 0 {
                    *n -= 1;
                    continue;
                }
            }
            let raiz: String = p
                .chars()
                .take(3.max(p.chars().count().saturating_sub(3)))
                .collect();
            if !(p.chars().count() >= 3
                && antes.iter().any(|o| {
                    o.chars().count() >= 3
                        && (o.starts_with(&raiz) || p.starts_with(o) || p.ends_with(o))
                }))
            {
                novas += 1
            }
        }
        let toleradas = 2.max((antes.len() as f64 * 0.12).round() as usize);
        if novas > toleradas {
            return Some(if depois.len() > antes.len() + toleradas {
                Recusa::Respondeu
            } else {
                Recusa::Acrescentou
            });
        }
        (antes.len() >= 8 && 2 * depois.len() < antes.len()).then_some(Recusa::CortouDemais)
    }
}
