use crate::{Palavras, RegrasFala};
enum Passo {
    Par(usize, usize),
    Some(usize),
    Entra(usize),
}
fn alinhar(a: &[String], b: &[String]) -> Vec<Passo> {
    let m = b.len() + 1;
    let mut d = vec![0u32; (a.len() + 1) * m];
    for (i, r) in d.chunks_exact_mut(m).enumerate() {
        r[0] = i as u32
    }
    for (j, v) in d[..m].iter_mut().enumerate() {
        *v = j as u32
    }
    for (i, x) in a.iter().enumerate() {
        for (j, y) in b.iter().enumerate() {
            let k = (i + 1) * m + j + 1;
            d[k] = (d[k - 1] + 1)
                .min(d[k - m] + 1)
                .min(d[k - m - 1] + u32::from(x != y));
        }
    }
    let (mut i, mut j) = (a.len(), b.len());
    let mut passos = vec![];
    while i + j > 0 {
        let k = i * m + j;
        if i > 0 && j > 0 && d[k] == d[k - m - 1] + u32::from(a[i - 1] != b[j - 1]) {
            i -= 1;
            j -= 1;
            passos.push(Passo::Par(i, j))
        } else if i > 0 && d[k] == d[k - m] + 1 {
            i -= 1;
            passos.push(Passo::Some(i))
        } else {
            j -= 1;
            passos.push(Passo::Entra(j))
        }
    }
    passos.reverse();
    passos
}
fn forca(s: &str) -> u8 {
    if s.contains('\n') {
        4
    } else if s.contains(['.', '!', '?', '…']) {
        3
    } else if s.contains([':', ';']) {
        2
    } else {
        u8::from(s.contains(','))
    }
}
pub struct Fidelidade;
impl Fidelidade {
    pub fn fundir(original: &str, modelo: &str) -> String {
        let a: Vec<_> = Palavras::ocorrencias(original).collect();
        if a.is_empty() {
            return String::new();
        }
        let b: Vec<_> = Palavras::ocorrencias(modelo).collect();
        if b.is_empty() {
            return original.into();
        }
        let na = Palavras::normalizadas(original);
        let nb = Palavras::normalizadas(modelo);
        let passos = alinhar(&na, &nb);
        let vao = |k: usize| {
            &original[if k == 0 { 0 } else { a[k - 1].end() }
                ..a.get(k).map_or(original.len(), |p| p.start())]
        };
        let antes = |j: usize| &modelo[if j == 0 { 0 } else { b[j - 1].end() }..b[j].start()];
        let sinal = |k: usize| {
            let pre = &original[..a[k].start()];
            let mut chars = pre.chars().rev();
            match chars.next() {
                Some(c @ ('+' | '-' | '−'))
                    if a[k].as_str().starts_with(char::is_numeric)
                        && chars.next().is_none_or(char::is_whitespace) =>
                {
                    Some(c)
                }
                _ => None,
            }
        };
        let mut perto = vec![false; a.len()];
        for i in 0..a.len() {
            for expr in [
                "nao espera",
                "quer dizer",
                "alias",
                "ou melhor",
                "minto",
                "corrigindo",
                "perdao",
                "digo",
                "melhor dizendo",
            ] {
                let e: Vec<_> = expr.split_whitespace().collect();
                if na[i..].starts_with(&e.iter().map(|s| s.to_string()).collect::<Vec<_>>()) {
                    let mut inicio = i;
                    while inicio > i.saturating_sub(8) && forca(vao(inicio)) < 3 {
                        inicio -= 1
                    }
                    perto[inicio..i + e.len()].fill(true)
                }
            }
        }
        let pode_cortar = |k: usize| {
            let p = &*na[k];
            (matches!(p, "ne" | "ahn" | "hum" | "hm" | "uhm" | "eh" | "ah")
                || a[k].as_str().to_lowercase() == "hã")
                || matches!(
                    p,
                    "ta" | "entao"
                        | "assim"
                        | "tipo"
                        | "bom"
                        | "bem"
                        | "olha"
                        | "ai"
                        | "sabe"
                        | "entendeu"
                        | "cara"
                        | "ok"
                        | "enfim"
                ) && (k == 0 || forca(vao(k)) > 0)
                    && (k + 1 == a.len() || forca(vao(k + 1)) > 0)
        };
        let mut saida = String::new();
        let mut pendente = "";
        let mut restaurou = false;
        let mut proximo = 0;
        let mut i = 0;
        while i < passos.len() {
            match passos[i] {
                Passo::Par(k, j) => {
                    let mut sep = antes(j);
                    if restaurou {
                        sep = vao(k);
                        for q in proximo..k {
                            if forca(vao(q)) > forca(sep) {
                                sep = vao(q)
                            }
                        }
                        if sep.is_empty() {
                            sep = " "
                        }
                    }
                    restaurou = false;
                    if forca(pendente) > forca(sep) {
                        sep = pendente
                    }
                    pendente = "";
                    if saida.is_empty() {
                        saida.extend(sep.chars().filter(|c| "\"“(¿¡".contains(*c)))
                    } else {
                        saida.push_str(if sep.is_empty() { " " } else { sep })
                    }
                    if let Some(s) = sinal(k) {
                        if !saida.ends_with(s) {
                            saida.push(s)
                        }
                    }
                    let mut chars = a[k].as_str().chars();
                    let primeiro = chars.next().unwrap();
                    if primeiro.is_lowercase() && b[j].as_str().starts_with(char::is_uppercase) {
                        saida.extend(primeiro.to_uppercase());
                        saida.push_str(chars.as_str())
                    } else {
                        saida.push_str(a[k].as_str())
                    }
                    proximo = k + 1;
                }
                Passo::Entra(j) => {
                    if pendente.is_empty() || forca(antes(j)) > forca(pendente) {
                        pendente = antes(j)
                    }
                }
                Passo::Some(inicio) => {
                    let mut fim = inicio + 1;
                    while matches!(passos.get(i+1),Some(Passo::Some(k)) if forca(vao(*k))<3) {
                        fim += 1;
                        i += 1
                    }
                    let ks = inicio..fim;
                    let anterior = inicio.checked_sub(1).filter(|&k| perto[k]);
                    let corte = ks.clone().all(|k| {
                        pode_cortar(k) || perto[k] || anterior.is_some_and(|q| na[q] == na[k])
                    });
                    let n = ks.len();
                    let repete = (2..=8).contains(&n)
                        && (na[fim..na.len().min(fim + n + 2)]
                            .windows(n)
                            .any(|w| w == &na[ks.clone()])
                            || na[inicio.saturating_sub(n + 2)..inicio]
                                .windows(n)
                                .any(|w| w == &na[ks.clone()]));
                    if !corte && !repete {
                        pendente = "";
                        for k in ks {
                            if !saida.is_empty() {
                                let sep = vao(k);
                                if sep.is_empty() {
                                    saida.push(' ')
                                } else {
                                    saida.extend(
                                        sep.chars()
                                            .filter(|c| sinal(k).is_none() || !"-+−".contains(*c)),
                                    )
                                }
                            }
                            if let Some(s) = sinal(k) {
                                saida.push(s)
                            }
                            saida.push_str(a[k].as_str());
                            proximo = k + 1
                        }
                        restaurou = true
                    }
                }
            }
            i += 1
        }
        saida.push_str(&modelo[b.last().unwrap().end()..]);
        RegrasFala::arrumar_espacos(&saida)
    }
}
