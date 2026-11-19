//! Contagem de palavras igual à do Swift (`Palavras.ocorrencias`): letras e números, com hífen e
//! apóstrofo no meio da palavra. Usada no limite de tokens da limpeza e no WER.

fn eh_letra(c: char) -> bool {
    c.is_alphabetic() || c.is_numeric()
}

/// Palavras do texto, na ordem.
pub fn ocorrencias(texto: &str) -> Vec<&str> {
    let cs: Vec<(usize, char)> = texto.char_indices().collect();
    let mut saida = Vec::new();
    let mut inicio: Option<usize> = None;
    for (k, &(i, c)) in cs.iter().enumerate() {
        let proximo_letra = cs.get(k + 1).map(|&(_, p)| eh_letra(p)).unwrap_or(false);
        let dentro = eh_letra(c)
            || ((c == '-' || c == '\'' || c == '’') && inicio.is_some() && proximo_letra);
        if dentro {
            if inicio.is_none() {
                inicio = Some(i);
            }
        } else if let Some(ini) = inicio.take() {
            saida.push(&texto[ini..i]);
        }
    }
    if let Some(ini) = inicio {
        saida.push(&texto[ini..]);
    }
    saida
}

pub fn contar(texto: &str) -> usize {
    ocorrencias(texto).len()
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn conta_como_o_swift() {
        assert_eq!(contar(""), 0);
        assert_eq!(contar("Olá, mundo!"), 2);
        assert_eq!(
            ocorrencias("guarda-chuva d'água - solto"),
            vec!["guarda-chuva", "d'água", "solto"]
        );
        assert_eq!(ocorrencias("fim-"), vec!["fim"]);
        assert_eq!(ocorrencias("são 3 horas"), vec!["são", "3", "horas"]);
    }
}
