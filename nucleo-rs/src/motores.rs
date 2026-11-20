//! Interfaces dos motores, traduzidas dos protocolos Swift (`Limpador.swift`,
//! `Lancamento/ModeloLancamento.swift`). O crate `motores` implementa; o núcleo só usa.
use std::path::Path;

/// Taxa de todo o PCM do app: 16 kHz, mono, f32.
pub const TAXA: u32 = 16_000;

#[derive(Debug, Clone, PartialEq)]
pub struct PalavraReconhecida {
    pub texto: String,
    pub inicio: f64,
    pub fim: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FaixaDeVoz {
    pub voz: String,
    pub inicio: f64,
    pub fim: f64,
}

#[derive(Debug, Clone, Default)]
pub struct PedidoLimpeza {
    pub transcricao: String,
    pub anterior: String,
    pub vocabulario: Vec<String>,
}

pub type Erro = Box<dyn std::error::Error + Send + Sync>;

/// Ditado: amostras de um trecho, devolve o texto (Swift `Transcritor`).
pub trait Transcritor: Send + Sync {
    fn nome(&self) -> &str;
    fn transcrever(&self, amostras: &[f32]) -> Result<String, Erro>;
}

/// Arquivos e reuniões: palavras com tempo absoluto (Swift `ReconhecedorDeFala`).
/// `fim_da_fala` fecha a cauda: nenhuma palavra dura além do áudio real.
pub trait ReconhecedorDeFala: Send + Sync {
    fn palavras(
        &self,
        amostras: &[f32],
        inicio: f64,
        fim_da_fala: Option<f64>,
    ) -> Result<Vec<PalavraReconhecida>, Erro>;
}

/// Separação de falantes sobre um WAV da faixa inteira (Swift `IdentificadorDeFalantes`).
pub trait IdentificadorDeFalantes: Send + Sync {
    fn faixas(&self, wav: &Path) -> Result<Vec<FaixaDeVoz>, Erro>;
}

/// Limpeza por modelo local (Swift `Limpador`).
pub trait Limpador: Send + Sync {
    fn nome(&self) -> &str;
    fn limpar(&self, pedido: &PedidoLimpeza) -> Result<String, Erro>;
}
