//! Separação de falantes offline do sherpa-onnx: segmentação pyannote 3.0 + embeddings CAM++
//! + agrupamento. Equivale à segunda via do Swift (`diarizarPorAgrupamento`, prefixo `offline-`).
use crate::audio::{caminho_c, ler_wav};
use crate::catalogo::FALANTES;
use crate::{erro, Erro, FaixaDeVoz, IdentificadorDeFalantes};
use sherpa_onnx::{
    FastClusteringConfig, OfflineSpeakerDiarization, OfflineSpeakerDiarizationConfig,
    OfflineSpeakerSegmentationModelConfig, OfflineSpeakerSegmentationPyannoteModelConfig,
    SpeakerEmbeddingExtractorConfig,
};
use std::path::Path;

/// Limiar de distância do agrupamento quando o número de falantes é desconhecido.
/// Medido no corpus sintético (ver `medir --falantes`); o padrão do sherpa-onnx é 0,5.
pub const LIMIAR_PADRAO: f32 = 0.5;

pub struct Diarizador {
    motor: OfflineSpeakerDiarization,
}

impl Diarizador {
    /// `pasta` com os arquivos de [`FALANTES`]. `falantes`: número conhecido, ou `None` para estimar.
    pub fn carregar(
        pasta: &Path,
        falantes: Option<u32>,
        limiar: f32,
        threads: usize,
    ) -> Result<Self, Erro> {
        for a in FALANTES.arquivos {
            if !pasta.join(a.arquivo).is_file() {
                return Err(erro(format!(
                    "modelo de falantes incompleto: falta {}",
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
        } as i32;
        let config = OfflineSpeakerDiarizationConfig {
            segmentation: OfflineSpeakerSegmentationModelConfig {
                pyannote: OfflineSpeakerSegmentationPyannoteModelConfig {
                    model: Some(caminho_c(&pasta.join(FALANTES.arquivos[0].arquivo))?),
                    ..Default::default()
                },
                num_threads: threads,
                provider: Some("cpu".into()),
                ..Default::default()
            },
            embedding: SpeakerEmbeddingExtractorConfig {
                model: Some(caminho_c(&pasta.join(FALANTES.arquivos[1].arquivo))?),
                num_threads: threads,
                provider: Some("cpu".into()),
                ..Default::default()
            },
            clustering: FastClusteringConfig {
                num_clusters: falantes.map(|n| n as i32).unwrap_or(-1),
                threshold: limiar,
                ..Default::default()
            },
            ..Default::default()
        };
        let motor = OfflineSpeakerDiarization::create(&config)
            .ok_or_else(|| erro("o sherpa-onnx não carregou a diarização"))?;
        Ok(Self { motor })
    }

    /// Faixas a partir de PCM 16 kHz mono.
    pub fn faixas_de(&self, amostras: &[f32]) -> Result<Vec<FaixaDeVoz>, Erro> {
        if amostras.len() < crate::TAXA as usize / 2 {
            return Ok(Vec::new());
        }
        let r = self
            .motor
            .process(amostras)
            .ok_or_else(|| erro("a diarização falhou"))?;
        Ok(r.sort_by_start_time()
            .into_iter()
            .filter(|s| s.end > s.start)
            .map(|s| FaixaDeVoz {
                voz: format!("offline-{}", s.speaker),
                inicio: s.start as f64,
                fim: s.end as f64,
            })
            .collect())
    }
}

impl IdentificadorDeFalantes for Diarizador {
    fn faixas(&self, wav: &Path) -> Result<Vec<FaixaDeVoz>, Erro> {
        self.faixas_de(&ler_wav(wav)?)
    }
}
