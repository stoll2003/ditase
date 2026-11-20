//! Implementações dos motores: sherpa-onnx (Parakeet, diarização) e llama-server.
//!
//! - [`fala::Parakeet`]: `Transcritor` (ditado) e `ReconhecedorDeFala` (arquivos e reuniões).
//! - [`falantes::Diarizador`]: `IdentificadorDeFalantes` (pyannote 3.0 + CAM++).
//! - [`llama::ServidorLlama`] e [`llama::LimpadorLlama`]: `Limpador` por llama-server local.
//! - [`catalogo`]: modelos e binários com URL fixada em revisão, tamanho e SHA-256.
pub mod audio;
mod canal_llama;
pub mod catalogo;
pub mod fala;
pub mod falantes;
pub mod llama;
pub mod medicao;
pub mod palavras;

pub use nucleo::motores::{
    Erro, FaixaDeVoz, IdentificadorDeFalantes, Limpador, PalavraReconhecida, PedidoLimpeza,
    ReconhecedorDeFala, Transcritor, TAXA,
};

pub(crate) fn erro(msg: impl Into<String>) -> Erro {
    msg.into().into()
}

/// Pasta de modelos para testes e medição: `DITASE_MODELOS` ou `/tmp/ditase-motores-modelos`
/// (no Windows, `%TEMP%\ditase-motores-modelos`).
pub fn pasta_de_teste() -> std::path::PathBuf {
    if let Some(p) = std::env::var_os("DITASE_MODELOS") {
        return p.into();
    }
    let base = if cfg!(unix) {
        std::path::PathBuf::from("/tmp")
    } else {
        std::env::temp_dir()
    };
    base.join("ditase-motores-modelos")
}
