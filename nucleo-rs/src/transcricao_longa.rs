use std::collections::HashMap;
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TrechoLongo {
    pub id: String,
    pub inicio: f64,
    pub fim: f64,
    pub voz: String,
    pub texto: String,
    pub parcial: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voz_manual: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origem: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub divergente: Option<bool>,
}
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct VozLonga {
    pub id: String,
    pub nome: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nome_manual: Option<bool>,
}
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TranscricaoLonga {
    pub id: String,
    pub nome: String,
    pub criado_em: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duracao: Option<f64>,
    pub vozes: Vec<VozLonga>,
    pub trechos: Vec<TrechoLongo>,
    pub baixa_confianca: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bruto: Option<String>,
    pub fase: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progresso: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub motivo: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub processamento: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fonte: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub atraso: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resumo: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resumo_gerado: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oculto: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub erro_captura: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub erro_processamento: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aviso_captura: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub erro_resumo: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resumo_cancelado: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unioes_de_voz: Option<HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub palavras: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previa: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lacunas: Option<Vec<PeriodoDeAudio>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pendentes: Option<Vec<PeriodoDeAudio>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub falantes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub etapa: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aviso: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub segundos_pendentes: Option<f64>,
}
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PeriodoDeAudio {
    pub inicio: f64,
    pub fim: f64,
    pub fonte: String,
    pub motivo: String,
}
