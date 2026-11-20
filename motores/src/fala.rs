//! Parakeet TDT 0.6B v3 INT8 pelo sherpa-onnx (ONNX Runtime, CPU).
//!
//! Mesmo comportamento do Swift (`TranscritorParakeet`, `MotorLancamento.reconhecer`):
//! trecho curto é completado com silêncio até 1,1 s; com `fim_da_fala`, o modelo recebe 300 ms de
//! silêncio depois do áudio, mas nenhuma palavra passa do fim real da fala.
use crate::audio::caminho_c;
use crate::catalogo::PARAKEET;
use crate::{erro, Erro, PalavraReconhecida, ReconhecedorDeFala, Transcritor, TAXA};
use sherpa_onnx::{
    OfflineModelConfig, OfflineRecognizer, OfflineRecognizerConfig, OfflineTransducerModelConfig,
};
use std::path::Path;

/// Mínimo que o modelo aceita bem (igual ao Swift: 17.600 amostras).
const MINIMO: usize = 17_600;
/// Silêncio acrescentado para fechar a cauda quando há `fim_da_fala` (300 ms).
const SILENCIO_DA_CAUDA: usize = 4_800;

pub struct Parakeet {
    reconhecedor: OfflineRecognizer,
}

#[derive(Debug, Clone, PartialEq)]
struct Token {
    texto: String,
    inicio: f64,
    fim: f64,
}

impl Parakeet {
    /// `pasta` com os arquivos de [`PARAKEET`]. `threads` 0 escolhe pela máquina (até 4).
    pub fn carregar(pasta: &Path, threads: usize) -> Result<Self, Erro> {
        Self::carregar_com_busca(pasta, threads, false)
    }

    /// Comparação das buscas no corpus, usando os mesmos pesos e amostras.
    pub fn carregar_com_busca(pasta: &Path, threads: usize, beam: bool) -> Result<Self, Erro> {
        for a in PARAKEET.arquivos {
            if !pasta.join(a.arquivo).is_file() {
                return Err(erro(format!(
                    "modelo de fala incompleto: falta {}",
                    a.arquivo
                )));
            }
        }
        let threads = if threads == 0 {
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(2)
                .clamp(1, 4)
        } else {
            threads
        };
        let p = |n: &str| caminho_c(&pasta.join(n)).map(Some);
        let config = OfflineRecognizerConfig {
            model_config: OfflineModelConfig {
                transducer: OfflineTransducerModelConfig {
                    encoder: p("encoder.int8.onnx")?,
                    decoder: p("decoder.int8.onnx")?,
                    joiner: p("joiner.int8.onnx")?,
                },
                tokens: p("tokens.txt")?,
                model_type: Some("nemo_transducer".into()),
                num_threads: threads as i32,
                provider: Some("cpu".into()),
                ..Default::default()
            },
            decoding_method: Some(
                if beam {
                    "modified_beam_search"
                } else {
                    "greedy_search"
                }
                .into(),
            ),
            ..Default::default()
        };
        let reconhecedor = OfflineRecognizer::create(&config)
            .ok_or_else(|| erro("o sherpa-onnx não carregou o Parakeet"))?;
        Ok(Self { reconhecedor })
    }

    fn decodificar(&self, amostras: &[f32]) -> Result<(String, Vec<Token>), Erro> {
        let fluxo = self.reconhecedor.create_stream();
        fluxo.accept_waveform(TAXA as i32, amostras);
        self.reconhecedor.decode(&fluxo);
        let r = fluxo
            .get_result()
            .ok_or_else(|| erro("o Parakeet não devolveu resultado"))?;
        let inicios = r.timestamps.unwrap_or_default();
        let duracoes = r.durations.unwrap_or_default();
        let total = amostras.len() as f64 / TAXA as f64;
        let mut tokens = Vec::with_capacity(r.tokens.len());
        for (i, t) in r.tokens.into_iter().enumerate() {
            let Some(&ini) = inicios.get(i) else { break };
            let ini = ini as f64;
            // TDT dá a duração de cada token; sem ela, vai até o próximo token (máximo 0,5 s).
            let fim = match duracoes.get(i) {
                Some(&d) if d > 0.0 => ini + d as f64,
                _ => inicios
                    .get(i + 1)
                    .map(|&p| p as f64)
                    .unwrap_or(total)
                    .min(ini + 0.5),
            };
            tokens.push(Token {
                texto: t,
                inicio: ini,
                fim,
            });
        }
        Ok((r.text.trim().to_string(), tokens))
    }
}

/// Junta os tokens BPE em palavras: token que começa com `▁` (ou espaço) abre palavra nova.
/// `▁` isolado abre a palavra e o texto vem nos tokens seguintes.
fn palavras_dos_tokens(tokens: &[Token]) -> Vec<Token> {
    let mut saida: Vec<Token> = Vec::new();
    for t in tokens {
        let limpo = t.texto.trim_start_matches(['▁', ' ']);
        let novo = limpo.len() != t.texto.len() || saida.is_empty();
        match saida.last_mut() {
            Some(u) if !novo => {
                u.texto.push_str(limpo);
                u.fim = u.fim.max(t.fim);
            }
            _ => saida.push(Token {
                texto: limpo.to_string(),
                inicio: t.inicio,
                fim: t.fim,
            }),
        }
    }
    saida.retain(|p| !p.texto.trim().is_empty());
    saida
}

fn tem_som(amostras: &[f32]) -> bool {
    amostras.iter().any(|a| a.abs() > 0.002)
}

impl Transcritor for Parakeet {
    fn nome(&self) -> &str {
        "parakeet-tdt-0.6b-v3"
    }

    fn transcrever(&self, amostras: &[f32]) -> Result<String, Erro> {
        if amostras.iter().all(|&a| a == 0.0) {
            return Ok(String::new());
        }
        if amostras.len() >= MINIMO {
            return Ok(self.decodificar(amostras)?.0);
        }
        let mut entrada = vec![0.0; MINIMO];
        entrada[..amostras.len()].copy_from_slice(amostras);
        Ok(self.decodificar(&entrada)?.0)
    }
}

impl ReconhecedorDeFala for Parakeet {
    fn palavras(
        &self,
        amostras: &[f32],
        inicio: f64,
        fim_da_fala: Option<f64>,
    ) -> Result<Vec<PalavraReconhecida>, Erro> {
        if amostras.is_empty() || !tem_som(amostras) {
            return Ok(Vec::new());
        }
        let silencio = if fim_da_fala.is_some() {
            SILENCIO_DA_CAUDA
        } else {
            0
        };
        let completar = silencio.max(MINIMO.saturating_sub(amostras.len()));
        let (texto, tokens) = if completar == 0 {
            self.decodificar(amostras)?
        } else {
            let mut entrada = vec![0.0; amostras.len() + completar];
            entrada[..amostras.len()].copy_from_slice(amostras);
            self.decodificar(&entrada)?
        };
        let duracao =
            (amostras.len() as f64 / TAXA as f64).min(fim_da_fala.unwrap_or(f64::INFINITY));
        Ok(montar(texto, &tokens, inicio, duracao))
    }
}

/// Palavras com tempo absoluto, limitadas à `duracao` real (regra do Swift).
fn montar(texto: String, tokens: &[Token], inicio: f64, duracao: f64) -> Vec<PalavraReconhecida> {
    let palavras = palavras_dos_tokens(tokens);
    if palavras.is_empty() {
        if texto.is_empty() {
            return Vec::new();
        }
        return vec![PalavraReconhecida {
            texto,
            inicio,
            fim: inicio + duracao,
        }];
    }
    palavras
        .into_iter()
        .filter_map(|p| {
            let de = p.inicio.max(0.0);
            let ate = duracao.min((de + 0.02).max(p.fim));
            (ate > de).then_some(PalavraReconhecida {
                texto: p.texto,
                inicio: inicio + de,
                fim: inicio + ate,
            })
        })
        .collect()
}

#[cfg(test)]
mod testes {
    use super::*;

    fn t(texto: &str, inicio: f64, fim: f64) -> Token {
        Token {
            texto: texto.into(),
            inicio,
            fim,
        }
    }

    #[test]
    fn junta_tokens_em_palavras() {
        let ts = [
            t("▁bo", 0.0, 0.1),
            t("m", 0.1, 0.2),
            t("▁dia", 0.3, 0.5),
            t("▁", 0.6, 0.62),
            t("Ana", 0.62, 0.9),
            t(".", 0.9, 0.95),
        ];
        let p = palavras_dos_tokens(&ts);
        assert_eq!(
            p.iter().map(|p| p.texto.as_str()).collect::<Vec<_>>(),
            ["bom", "dia", "Ana."]
        );
        assert_eq!((p[0].inicio, p[0].fim), (0.0, 0.2));
        assert_eq!(p[2].inicio, 0.6);
    }

    #[test]
    fn fim_da_fala_corta_a_cauda() {
        let ts = [
            t("▁um", 0.0, 0.3),
            t("▁dois", 0.5, 1.4),
            t("▁três", 1.5, 1.9),
        ];
        let p = montar("um dois três".into(), &ts, 10.0, 1.2);
        assert_eq!(p.len(), 2, "palavra que começa depois do fim some");
        assert_eq!(p[1].fim, 11.2);
        assert!(p.iter().all(|p| p.fim <= 11.2 && p.inicio >= 10.0));
    }

    #[test]
    fn sem_tempos_devolve_o_texto_inteiro() {
        let p = montar("olá".into(), &[], 2.0, 1.0);
        assert_eq!(
            p,
            vec![PalavraReconhecida {
                texto: "olá".into(),
                inicio: 2.0,
                fim: 3.0
            }]
        );
    }
}
