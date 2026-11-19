//! Apoio da medição: erro de palavras (WER) e memória de pico do processo.
use crate::palavras;

/// Normaliza para comparar: minúsculas, sem pontuação; acentos ficam (em pt-BR "e" e "é" são
/// palavras diferentes). Hífen vira separador, para "guarda-chuva" e "guarda chuva" empatarem.
pub fn normalizar(texto: &str) -> Vec<String> {
    palavras::ocorrencias(texto)
        .into_iter()
        .flat_map(|p| {
            p.split(['-', '\'', '’'])
                .map(str::to_lowercase)
                .collect::<Vec<_>>()
        })
        .filter(|p| !p.is_empty())
        .collect()
}

/// Distância de edição entre as palavras de referência e as obtidas:
/// (substituições + remoções + inserções, total de palavras da referência).
pub fn erros_de_palavras(referencia: &str, obtido: &str) -> (usize, usize) {
    let r = normalizar(referencia);
    let h = normalizar(obtido);
    let mut ant: Vec<usize> = (0..=h.len()).collect();
    for i in 1..=r.len() {
        let mut atual = vec![i; h.len() + 1];
        for j in 1..=h.len() {
            let custo = usize::from(r[i - 1] != h[j - 1]);
            atual[j] = (ant[j - 1] + custo).min(ant[j] + 1).min(atual[j - 1] + 1);
        }
        ant = atual;
    }
    (ant[h.len()], r.len())
}

/// Memória residente de pico do processo, em bytes.
pub fn ram_de_pico() -> u64 {
    #[cfg(unix)]
    unsafe {
        let mut u: libc::rusage = std::mem::zeroed();
        libc::getrusage(libc::RUSAGE_SELF, &mut u);
        // macOS devolve bytes; Linux, kibibytes.
        if cfg!(target_os = "macos") {
            u.ru_maxrss as u64
        } else {
            u.ru_maxrss as u64 * 1024
        }
    }
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::ProcessStatus::{
            GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
        };
        use windows_sys::Win32::System::Threading::GetCurrentProcess;
        let mut c: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
        c.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        if GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb) != 0 {
            c.PeakWorkingSetSize as u64
        } else {
            0
        }
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn wer_basico() {
        assert_eq!(erros_de_palavras("Bom dia, Ana.", "bom dia ana"), (0, 3));
        assert_eq!(erros_de_palavras("bom dia ana", "bom tarde ana"), (1, 3));
        assert_eq!(erros_de_palavras("bom dia ana", "bom ana"), (1, 3));
        assert_eq!(erros_de_palavras("bom dia", "bom dia mesmo"), (1, 2));
        assert_eq!(erros_de_palavras("é isso", "e isso"), (1, 2));
        assert_eq!(erros_de_palavras("guarda-chuva", "guarda chuva"), (0, 2));
    }

    #[test]
    fn ram_positiva() {
        assert!(ram_de_pico() > 1_000_000);
    }
}
