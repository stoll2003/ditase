//! Único ponto de contato da casca com o núcleo (`nucleo-rs`) e os motores (`motores`).
//!
//! O Dev Back está portando a API pública Swift `Nucleo` com os mesmos nomes em snake_case.
//! Cada função aqui diz qual chamada do núcleo ela espera. Enquanto o núcleo não tem a peça,
//! a função devolve [`Faltando`] (mutações) ou um valor vazio no formato do contrato (leituras),
//! para a interface abrir sem erro. Na integração, só este arquivo muda.
//!
//! Lista do que depende do núcleo e dos motores: [`PENDENCIAS`] (também sai no relatório de teste).
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

#[derive(Debug, Clone)]
pub struct Faltando(pub &'static str);

impl std::fmt::Display for Faltando {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Ainda não disponível nesta versão de teste do Windows: {}.", self.0)
    }
}
impl std::error::Error for Faltando {}

pub type Res<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

fn falta<T>(o_que: &'static str) -> Res<T> { Err(Box::new(Faltando(o_que))) }

/// Pendências de integração: (peça esperada, o que fica sem funcionar até lá).
pub const PENDENCIAS: &[(&str, &str)] = &[
    ("nucleo::Banco (abrir, inserir, atualizar, concluir, ditado, historico, apagar_ditado, para_estatisticas, gravar_trecho, interrompidos, termos, salvar_termo, apagar_termo, contar_uso, ultimo_concluido)", "histórico, dicionário, insights, recuperação de ditado interrompido"),
    ("nucleo::Estatisticas::calcular", "página Insights (vem zerada)"),
    ("nucleo::SessaoDitado + nucleo::AplicadorDicionario + nucleo::Segmentador", "transcrever o ditado (hoje grava o WAV e avisa)"),
    ("motores: Transcritor Parakeet (sherpa-onnx)", "reconhecedor de fala"),
    ("motores: Limpador llama-server + nucleo::LimpadorLlama", "limpeza por IA"),
    ("nucleo::BaixadorDeModelo + catálogo do modelo padrão", "baixar o modelo de IA pelo Início"),
    ("nucleo::lancamento (ArmazemLancamento, ProcessadorDeArquivo, ProcessadorDeReuniao, ColetorDeReuniao, DiarioReuniao, LeituraLancamento)", "Arquivos e Reunião (disponivel=false)"),
    ("motores: IdentificadorDeFalantes (diarização sherpa-onnx)", "falantes em arquivos e reuniões"),
    ("nucleo::ImportadorWispr (se houver banco equivalente no Windows)", "importar do Wispr Flow"),
];

/// Resultado de um ditado, espelho de `ResultadoDitado` do Swift.
#[derive(Debug, Clone, Default)]
pub struct ResultadoDitado {
    pub texto: String,
    pub texto_bruto: String,
    pub palavras: u32,
    pub duracao_fala: f64,
    pub limpador: String,
    pub trocas: Vec<i64>,
    pub falhas: u32,
    pub recusas: u32,
}

/// Metadados do ditado que a casca cria e completa (espelho de `nucleo::Ditado`).
#[derive(Debug, Clone, Default)]
pub struct Ditado {
    pub id: i64,
    pub iniciado_em: f64,
    pub duracao: f64,
    pub latencia: f64,
    pub app: String,
    pub app_nome: String,
    pub estado: String,
    pub audio: String,
    pub motor: String,
}

/// Sessão de um ditado: espera `nucleo::SessaoDitado` (alimentar, finalizar, cancelar, medidor.silencio).
pub trait SessaoDeDitado: Send {
    fn alimentar(&mut self, amostras: &[f32]);
    fn silencio(&self) -> f64;
    fn finalizar(self: Box<Self>) -> ResultadoDitado;
    fn cancelar(self: Box<Self>);
}

pub struct Integracao { banco: PathBuf }

impl Integracao {
    /// Espera `nucleo::Banco::abrir(caminho)`.
    pub fn abrir(banco: &Path) -> Self { Self { banco: banco.to_path_buf() } }
    pub fn caminho_do_banco(&self) -> &Path { &self.banco }

    // MARK: Histórico e dicionário (nucleo::Banco)

    /// `Banco::historico(limite, None, busca)` serializado como `Ditado[]` do contrato.
    pub fn historico(&self, _limite: usize, _busca: &str) -> Res<Value> { Ok(json!([])) }
    /// `Banco::termos()`.
    pub fn termos(&self) -> Res<Value> { Ok(json!([])) }
    /// `Banco::salvar_termo(Termo { id, termo, ouvido_como, .. })`, preservando usos, origem e criado_em do atual.
    pub fn salvar_termo(&self, _id: i64, _termo: &str, _ouvido_como: Vec<String>) -> Res<Value> { falta("dicionário (núcleo)") }
    /// `Banco::apagar_termo(id)`.
    pub fn apagar_termo(&self, _id: i64) -> Res<()> { falta("dicionário (núcleo)") }
    /// `Banco::ditado(id)`: devolve o nome do arquivo de áudio guardado, para apagar junto.
    pub fn audio_do_ditado(&self, _id: i64) -> Res<Option<String>> { Ok(None) }
    /// `Banco::apagar_ditado(id)`.
    pub fn apagar_ditado(&self, _id: i64) -> Res<()> { falta("histórico (núcleo)") }
    /// `Estatisticas::calcular(banco.para_estatisticas()?, agora, dias_no_grafico)`.
    pub fn estatisticas(&self, dias: u32) -> Res<Value> { Ok(estatisticas_vazias(dias)) }
    /// `Banco::ultimo_concluido()` → texto.
    pub fn ultimo_texto(&self) -> Res<Option<String>> { Ok(None) }
    /// `Banco::interrompidos()` + `trechos(id)`: recupera ditados que ficaram "gravando".
    pub fn recuperar_interrompidos(&self) -> Res<usize> { Ok(0) }

    // MARK: Ciclo do ditado

    /// `Banco::inserir(Ditado { estado: "gravando", .. })` → id. Sem núcleo: id local, nada persiste.
    pub fn inserir_ditado(&self, _d: &Ditado) -> Res<i64> {
        Ok(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(1))
    }
    /// `Banco::atualizar` / `Banco::concluir` com o resultado; `Banco::contar_uso(trocas)`.
    pub fn concluir_ditado(&self, _d: &Ditado, _r: &ResultadoDitado) -> Res<()> { falta("histórico (núcleo)") }
    /// `Banco::atualizar(estado = "falhou")`, mantendo o áudio para retranscrever.
    pub fn marcar_falha(&self, _d: &Ditado) -> Res<()> { falta("histórico (núcleo)") }
    /// `Banco::apagar_ditado(id)` para o ditado em silêncio ou cancelado.
    pub fn descartar_ditado(&self, _id: i64) -> Res<()> { Ok(()) }
    /// `SessaoDitado::new(transcritor, limpador, AplicadorDicionario::new(banco.termos()?, ehPalavraComum), config, ao_trecho)`.
    /// `limpeza` traz (porta do llama-server, modelo) quando a limpeza por IA está ligada.
    pub fn nova_sessao(&self, _id: i64, _limpeza: Option<(u16, String)>) -> Res<Box<dyn SessaoDeDitado>> {
        falta("transcrição do ditado (núcleo e motor de fala)")
    }
    /// Retranscrever: `Banco::ditado(id)` → áudio → nova sessão → `Banco::atualizar`.
    pub fn retranscrever(&self, _id: i64) -> Res<()> { falta("retranscrever (núcleo e motor de fala)") }

    // MARK: Modelos

    /// Reconhecedor carregado (`motores` Parakeet). Estado para a interface.
    pub fn reconhecedor(&self) -> (&'static str, String) { ("falhou", "Motor de fala ainda não integrado nesta versão de teste.".into()) }
    /// Modelo padrão de IA: `nucleo::BaixadorDeModelo` + `ModeloRemoto` do catálogo.
    pub fn modelo_ia(&self) -> Value {
        json!({"estado": "indisponivel", "arquivo": "Qwen3.5-2B-Q4_K_M.gguf", "tamanho": 0})
    }
    pub fn baixar_modelo(&self) -> Res<()> { falta("download do modelo de IA (núcleo)") }
    pub fn cancelar_modelo(&self) -> Res<()> { Ok(()) }
    /// `ImportadorWispr::importar(banco)`.
    pub fn importar_wispr(&self) -> Res<Value> { falta("importar do Wispr Flow") }
}

/// Mesmo formato de `Estatisticas` do contrato, tudo zero, com `porDia` dos últimos `dias`.
pub fn estatisticas_vazias(dias: u32) -> Value {
    let hoje = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0) / 86_400;
    let por_dia: Vec<Value> = (0..dias.clamp(7, 365) as i64).rev().map(|n| {
        let (a, m, d) = civil(hoje - n);
        json!({"dia": format!("{a:04}-{m:02}-{d:02}"), "palavras": 0, "ditados": 0, "minutos": 0})
    }).collect();
    json!({
        "totalPalavras": 0, "totalDitados": 0, "minutosFalados": 0, "ppmMedio": 0, "ppmDeCadaDitado": [],
        "minutosEconomizados": 0, "diasSeguidos": 0, "maiorSequencia": 0, "diasAtivos": 0, "palavrasHoje": 0,
        "palavrasNaSemana": 0, "palavrasNaSemanaAnterior": 0, "maiorDitadoPalavras": 0, "maiorDitadoMinutos": 0,
        "latenciaMediana": 0, "trocasDicionario": 0, "porDia": por_dia, "porHora": vec![0; 24], "porDiaDaSemana": vec![0; 7], "porApp": [],
    })
}

/// Dia civil (UTC) a partir de dias desde 1970 (algoritmo de Howard Hinnant).
fn civil(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// Serviço de arquivos e reuniões (espelho de `ServicoLancamento.swift`). A captura e a
/// decodificação são da casca (`audio`, `decodificacao`); o processamento, o armazenamento e
/// a recuperação são do núcleo.
pub struct Lancamento;

impl Lancamento {
    /// `ServicoLancamento::estado()`: sem núcleo, `disponivel: false` e listas vazias.
    pub fn estado(&self, microfone: bool) -> Value {
        json!({
            "arquivos": [], "anteriores": [], "recuperaveis": [],
            "permissoes": {"audioSistema": "concedida", "microfone": if microfone { "concedida" } else { "negada" }},
            "disponivel": false
        })
    }
    pub fn documento(&self, _id: &str) -> Res<Value> { falta("documentos de arquivos e reuniões (núcleo)") }
    pub fn procurar_arquivos(&self, _termo: &str) -> Res<Value> { Ok(json!([])) }
    pub fn recuperar_reuniao(&self, _id: &str) -> Res<()> { falta("recuperar reunião (núcleo)") }
    pub fn descartar_reuniao(&self, _id: &str) -> Res<()> { falta("descartar reunião (núcleo)") }
    /// `ProcessadorDeArquivo` com o PCM já decodificado pela casca. Devolve o id do primeiro arquivo.
    pub fn adicionar(&self, _arquivos: &[PathBuf]) -> Res<Option<String>> { falta("transcrever arquivos (núcleo e motores)") }
    pub fn fonte_para_repetir(&self, _id: &str) -> Res<Option<PathBuf>> { Ok(None) }
    pub fn repetir(&self, _id: &str, _arquivo: &Path) -> Res<()> { falta("refazer arquivo (núcleo)") }
    pub fn cancelar(&self, _id: &str) {}
    pub fn remover(&self, _id: &str) -> Res<()> { falta("remover (núcleo)") }
    pub fn desfazer(&self, _id: &str) -> Res<()> { falta("desfazer (núcleo)") }
    pub fn cancelar_fila(&self) {}
    /// `ColetorDeReuniao` alimentado pela captura da casca (sistema por loopback e microfone).
    pub fn iniciar_reuniao(&self, _fonte: &str) -> Res<()> { falta("reunião (núcleo e motores)") }
    pub fn pausar(&self, _id: &str) -> Res<()> { falta("reunião (núcleo)") }
    pub fn encerrar(&self, _id: &str) -> Res<()> { falta("reunião (núcleo)") }
    pub fn resumir(&self, _id: &str) -> Res<()> { falta("resumo (núcleo e IA local)") }
    pub fn cancelar_resumo(&self, _id: &str) {}
    pub fn alterar_titulo(&self, _id: &str, _nome: &str) -> Res<()> { falta("editar título (núcleo)") }
    pub fn renomear_voz(&self, _id: &str, _voz: &str, _nome: &str) -> Res<()> { falta("renomear falante (núcleo)") }
    pub fn reatribuir(&self, _id: &str, _trecho: &str, _voz: Option<&str>, _nome: Option<&str>) -> Res<()> { falta("reatribuir trecho (núcleo)") }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn calendario_civil() {
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(20_000), (2024, 10, 4));
    }

    #[test]
    fn estatisticas_vazias_no_formato_do_contrato() {
        let e = estatisticas_vazias(30);
        assert_eq!(e["porDia"].as_array().unwrap().len(), 30);
        assert_eq!(e["porHora"].as_array().unwrap().len(), 24);
        for campo in ["totalPalavras", "ppmDeCadaDitado", "porApp", "latenciaMediana", "porDiaDaSemana"] { assert!(e.get(campo).is_some(), "{campo}"); }
    }
}
