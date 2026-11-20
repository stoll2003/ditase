//! Ligação da casca com o núcleo (`nucleo`) e os motores (`motores`) no ditado e no histórico.
//!
//! Os motores entram pelo trait [`Motores`]: no app, [`crate::motores_locais::MotoresLocais`]
//! (Parakeet, diarização e llama-server); nos testes, dublês que não baixam modelo nenhum.
//! Arquivos e reuniões ficam em [`crate::lancamento`].
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
};

use nucleo::{
    motores::{IdentificadorDeFalantes, Limpador, ReconhecedorDeFala, Transcritor},
    AoTrecho, AplicadorDicionario, Banco, ConfigSessao, Estatisticas, Palavras, RegrasFala,
    SessaoDitado, Termo,
};
use serde_json::Value;

use crate::caminhos::nota;

pub type Res<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// O reconhecedor serve aos dois fluxos: ditado (`Transcritor`) e arquivos/reuniões (`ReconhecedorDeFala`).
pub trait Fala: Transcritor + ReconhecedorDeFala {}
impl<T: Transcritor + ReconhecedorDeFala + ?Sized> Fala for T {}

/// Por que o reconhecedor não está disponível. O texto (`Display`) é o que a pílula mostra.
#[derive(Debug, Clone, PartialEq)]
pub enum FalaIndisponivel {
    Ausente,
    Baixando { feito: u64, total: u64 },
    Carregando,
    Demorou,
    Falhou(String),
}

impl std::fmt::Display for FalaIndisponivel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ausente => f.write_str("O modelo de fala ainda não foi baixado."),
            Self::Baixando { feito, total } => write!(
                f,
                "Baixando o modelo de fala: {}%",
                (feito * 100).checked_div(*total).unwrap_or(0)
            ),
            Self::Carregando => {
                f.write_str("O modelo de fala está carregando. Tente em alguns segundos.")
            }
            Self::Demorou => f.write_str("O modelo de fala demorou demais para carregar."),
            Self::Falhou(m) => f.write_str(m),
        }
    }
}
impl std::error::Error for FalaIndisponivel {}

/// O que a casca precisa dos motores. Nada aqui baixa ou carrega modelo nos testes.
pub trait Motores: Send + Sync {
    /// Reconhecedor carregado. `esperar`: bloqueia enquanto o modelo ainda carrega (arquivos em fila).
    /// O erro é um texto para mostrar à pessoa ("Baixando o modelo de fala: 40%").
    fn fala(&self, esperar: bool) -> Result<Arc<dyn Fala>, FalaIndisponivel>;
    /// Os arquivos do modelo de fala estão no disco (mesmo que ainda carregando).
    fn fala_disponivel(&self) -> bool;
    /// Separação de falantes, quando o modelo existe.
    fn falantes(&self) -> Option<Arc<dyn IdentificadorDeFalantes>>;
    /// Limpeza por IA do ditado, só quando ligada nos ajustes e com modelo e servidor presentes.
    fn limpador(&self) -> Option<Arc<dyn Limpador>>;
    /// Começa a subir o servidor de IA no começo da gravação (o primeiro trecho não espera).
    fn aquecer(&self) {}
    /// Resumo da reunião por IA local disponível agora.
    fn resumo_disponivel(&self) -> bool;
    /// Até quatro tópicos a partir das linhas "Nome: fala".
    fn resumir(
        &self,
        linhas: &[String],
        cancelado: &dyn Fn() -> bool,
    ) -> Result<Vec<String>, String>;
    /// Estado do reconhecedor para a interface: (estado, detalhe).
    fn estado_reconhecedor(&self) -> (String, String);
    /// Objeto `limpador` do contrato (sem `ligado` e `modelo`, que vêm dos ajustes).
    fn estado_limpador(&self) -> (String, String);
    /// Objeto `modeloIA` do contrato (sem `aviso`).
    fn modelo_ia(&self) -> Value;
    fn baixar_modelo(&self) -> Result<(), String>;
    fn cancelar_modelo(&self);
    fn llama_instalado(&self) -> bool;
    /// Saída do app: para o servidor de IA (sem mexer nos ajustes).
    fn desligar(&self) {}
}

/// Resultado de um ditado, espelho de `nucleo::ResultadoDitado`.
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

impl From<nucleo::ResultadoDitado> for ResultadoDitado {
    fn from(r: nucleo::ResultadoDitado) -> Self {
        Self {
            texto: r.texto,
            texto_bruto: r.texto_bruto,
            palavras: r.palavras as u32,
            duracao_fala: r.duracao_fala,
            limpador: r.limpador,
            trocas: r.trocas.into_iter().map(|t| t.termo_id).collect(),
            falhas: r.falhas as u32,
            recusas: r.recusas as u32,
        }
    }
}

/// Metadados do ditado que o controlador cria e completa.
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

/// Sessão de um ditado (envolve `nucleo::SessaoDitado`).
pub trait SessaoDeDitado: Send {
    fn alimentar(&mut self, amostras: &[f32]);
    fn silencio(&self) -> f64;
    fn finalizar(self: Box<Self>) -> ResultadoDitado;
    fn cancelar(self: Box<Self>);
}

struct Sessao(SessaoDitado);

impl SessaoDeDitado for Sessao {
    fn alimentar(&mut self, amostras: &[f32]) {
        if let Err(e) = self.0.alimentar(amostras) {
            nota(format!("sessão: {e}"));
        }
    }
    fn silencio(&self) -> f64 {
        self.0.medidor().1
    }
    fn finalizar(self: Box<Self>) -> ResultadoDitado {
        match self.0.finalizar() {
            Ok(r) => r.into(),
            Err(e) => {
                nota(format!("sessão: {e}"));
                ResultadoDitado {
                    falhas: 1,
                    ..Default::default()
                }
            }
        }
    }
    fn cancelar(self: Box<Self>) {
        let mut s = self.0;
        s.cancelar();
    }
}

pub fn agora() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

fn json<T: serde::Serialize>(v: T) -> Res<Value> {
    Ok(serde_json::to_value(v)?)
}

pub struct Integracao {
    /// Onde a fila do ditado guarda o excedente de áudio (M6). `None`: só RAM.
    pasta_de_trabalho: Option<PathBuf>,
    banco: Arc<Mutex<Banco>>,
    caminho: PathBuf,
    pub motores: Arc<dyn Motores>,
}

impl Integracao {
    pub fn abrir(caminho: &Path, motores: Arc<dyn Motores>) -> Res<Self> {
        Ok(Self {
            pasta_de_trabalho: None,
            banco: Arc::new(Mutex::new(Banco::new(caminho)?)),
            caminho: caminho.to_path_buf(),
            motores,
        })
    }
    pub fn caminho_do_banco(&self) -> &Path {
        &self.caminho
    }

    /// Pasta privada do app para o excedente da fila do ditado, em vez da pasta temporária do sistema (M6).
    pub fn com_pasta_de_trabalho(mut self, pasta: PathBuf) -> Self {
        if std::fs::create_dir_all(&pasta).is_ok() {
            self.pasta_de_trabalho = Some(pasta);
        }
        self
    }
    fn b(&self) -> MutexGuard<'_, Banco> {
        self.banco.lock().unwrap_or_else(|e| e.into_inner())
    }

    // MARK: Histórico e dicionário

    pub fn historico(&self, limite: usize, busca: &str) -> Res<Value> {
        json(self.b().historico(limite as i64, None, busca)?)
    }
    pub fn termos(&self) -> Res<Value> {
        json(self.b().termos()?)
    }

    /// Termo existente preserva usos, origem e data de criação (como no Mac).
    pub fn salvar_termo(&self, id: i64, termo: &str, ouvido_como: Vec<String>) -> Res<Value> {
        let mut b = self.b();
        let mut novo = Termo {
            id,
            termo: termo.to_string(),
            ouvido_como,
            ..Default::default()
        };
        if id != 0 {
            if let Some(atual) = b.termos()?.into_iter().find(|t| t.id == id) {
                novo.usos = atual.usos;
                novo.origem = atual.origem;
                novo.criado_em = atual.criado_em;
            }
        }
        json(b.salvar(novo)?)
    }
    pub fn apagar_termo(&self, id: i64) -> Res<()> {
        Ok(self.b().apagar_termo(id)?)
    }
    pub fn audio_do_ditado(&self, id: i64) -> Res<Option<String>> {
        Ok(self
            .b()
            .ditado(id)?
            .map(|d| d.audio)
            .filter(|a| !a.is_empty()))
    }
    pub fn apagar_ditado(&self, id: i64) -> Res<()> {
        Ok(self.b().apagar_ditado(id)?)
    }
    pub fn estatisticas(&self, dias: u32) -> Res<Value> {
        let ditados = self.b().para_estatisticas()?;
        json(Estatisticas::calcular(
            &ditados,
            agora(),
            dias.clamp(7, 365) as usize,
        ))
    }
    pub fn ultimo_texto(&self) -> Res<Option<String>> {
        Ok(self.b().ultimo_concluido()?.map(|d| d.texto))
    }

    /// Ditado que ficou "gravando" porque o app fechou no meio: o que já estava transcrito volta ao
    /// histórico como "recuperado"; sem texto e com áudio guardado, fica "falhou" para retranscrever.
    pub fn recuperar_interrompidos(&self, audios: &Path) -> Res<usize> {
        let mut b = self.b();
        let mut recuperados = 0;
        for mut d in b.interrompidos()? {
            let trechos = match b.trechos(d.id) {
                Ok(t) => t,
                Err(e) => {
                    nota(format!(
                        "recuperação: trechos do ditado {} ilegíveis: {e}",
                        d.id
                    ));
                    continue;
                }
            };
            if trechos.is_empty() {
                if !d.audio.is_empty() && audios.join(&d.audio).is_file() {
                    d.estado = "falhou".into();
                    b.atualizar(&d)?;
                } else {
                    b.apagar_ditado(d.id)?;
                }
                continue;
            }
            d.texto_bruto = trechos.join(" ");
            d.texto = RegrasFala::filtrar(&d.texto_bruto);
            d.palavras = Palavras::contar(&d.texto) as i64;
            d.estado = "recuperado".into();
            d.limpador = "regras".into();
            match b.concluir(&d) {
                Ok(()) => {
                    recuperados += 1;
                    nota(format!(
                        "ditado {} recuperado com {} palavras",
                        d.id, d.palavras
                    ));
                }
                Err(e) => nota(format!(
                    "recuperação: ditado {} não foi gravado, reserva mantida: {e}",
                    d.id
                )),
            }
        }
        Ok(recuperados)
    }

    // MARK: Ciclo do ditado

    pub fn inserir_ditado(&self, d: &Ditado) -> Res<i64> {
        let n = nucleo::Ditado {
            iniciado_em: d.iniciado_em,
            app: d.app.clone(),
            app_nome: d.app_nome.clone(),
            estado: "gravando".into(),
            motor: d.motor.clone(),
            origem: "ditado".into(),
            audio: d.audio.clone(),
            ..Default::default()
        };
        Ok(self.b().inserir(&n, None)?)
    }

    pub fn definir_audio(&self, id: i64, audio: &str) -> Res<()> {
        let b = self.b();
        if let Some(mut d) = b.ditado(id)? {
            d.audio = audio.to_string();
            b.atualizar(&d)?;
        }
        Ok(())
    }

    fn preencher(atual: Option<nucleo::Ditado>, d: &Ditado, r: &ResultadoDitado) -> nucleo::Ditado {
        let mut n = atual.unwrap_or_else(|| nucleo::Ditado {
            id: d.id,
            iniciado_em: d.iniciado_em,
            origem: "ditado".into(),
            ..Default::default()
        });
        n.duracao = d.duracao;
        n.duracao_fala = r.duracao_fala;
        n.latencia = d.latencia;
        n.texto_bruto = r.texto_bruto.clone();
        n.texto = r.texto.clone();
        n.palavras = r.palavras as i64;
        n.limpador = r.limpador.clone();
        n.trocas_dicionario = r.trocas.len() as i64;
        n.estado = d.estado.clone();
        n.audio = d.audio.clone();
        n
    }

    /// Grava o resultado e apaga a reserva de trechos na mesma transação (`Banco::concluir`).
    pub fn concluir_ditado(&self, d: &Ditado, r: &ResultadoDitado) -> Res<()> {
        let mut b = self.b();
        let n = Self::preencher(b.ditado(d.id)?, d, r);
        b.concluir(&n)?;
        if !r.trocas.is_empty() {
            b.contar_uso(&r.trocas)?;
        }
        Ok(())
    }

    /// Reconhecedor falhou: o ditado fica "falhou" com o áudio para retranscrever.
    pub fn marcar_falha(&self, d: &Ditado) -> Res<()> {
        let b = self.b();
        let mut n = Self::preencher(b.ditado(d.id)?, d, &ResultadoDitado::default());
        n.estado = "falhou".into();
        Ok(b.atualizar(&n)?)
    }

    pub fn descartar_ditado(&self, id: i64) -> Res<()> {
        Ok(self.b().apagar_ditado(id)?)
    }

    fn sessao(
        &self,
        id: Option<i64>,
        limpeza: bool,
        esperar: bool,
    ) -> Result<SessaoDitado, String> {
        let fala = self.motores.fala(esperar).map_err(|e| e.to_string())?;
        let transcritor: Arc<dyn Transcritor> = fala;
        let limpador = if limpeza {
            self.motores.limpador()
        } else {
            None
        };
        let termos = self.b().termos().map_err(|e| e.to_string())?;
        let dicionario = AplicadorDicionario::new(termos, None);
        let ao_trecho: Option<AoTrecho> = id.map(|id| {
            let banco = self.banco.clone();
            Arc::new(move |ordem: usize, bruto: &str| {
                let b = banco.lock().unwrap_or_else(|e| e.into_inner());
                if let Err(e) = b.gravar_trecho(id, ordem as i64, bruto) {
                    nota(format!(
                        "trecho {ordem} do ditado {id} não foi gravado: {e}"
                    ));
                }
            }) as AoTrecho
        });
        let config = ConfigSessao {
            pasta_de_trabalho: self.pasta_de_trabalho.clone(),
            ..ConfigSessao::default()
        };
        Ok(SessaoDitado::new(
            transcritor,
            limpador,
            dicionario,
            config,
            ao_trecho,
        ))
    }

    /// Sessão do ditado `id`: cada trecho bruto vai para a reserva no banco antes do fim.
    pub fn nova_sessao(&self, id: i64, limpeza: bool) -> Result<Box<dyn SessaoDeDitado>, String> {
        Ok(Box::new(Sessao(self.sessao(Some(id), limpeza, false)?)))
    }

    /// Passa de novo pelo reconhecedor o áudio guardado de um ditado que falhou ou ficou parcial.
    /// Devolve o texto da pílula.
    pub fn retranscrever(
        &self,
        id: i64,
        audios: &Path,
        limpeza: bool,
    ) -> Result<&'static str, String> {
        let d = self
            .b()
            .ditado(id)
            .map_err(|e| e.to_string())?
            .ok_or("Ditado não encontrado")?;
        if d.audio.is_empty() || d.audio.contains(['/', '\\']) {
            return Err("O áudio desse ditado não existe mais".into());
        }
        let amostras = crate::decodificacao::decodificar(&audios.join(&d.audio))
            .map_err(|_| "O áudio desse ditado não existe mais".to_string())?;
        let mut sessao = self.sessao(None, limpeza, true)?;
        for bloco in amostras.chunks(16_000) {
            sessao.alimentar(bloco).map_err(|e| e.to_string())?;
        }
        let r = sessao.finalizar().map_err(|e| e.to_string())?;
        if r.texto.is_empty() {
            return Ok("Ainda não consegui transcrever");
        }
        let parcial = r.falhas > 0;
        let r: ResultadoDitado = r.into();
        let mut n = d;
        n.texto = r.texto.clone();
        n.texto_bruto = r.texto_bruto.clone();
        n.palavras = r.palavras as i64;
        n.duracao_fala = r.duracao_fala;
        n.limpador = r.limpador.clone();
        n.estado = if parcial { "parcial" } else { "concluido" }.into();
        self.b().atualizar(&n).map_err(|e| e.to_string())?;
        Ok(if parcial {
            "Parte do áudio não foi transcrita"
        } else {
            "Transcrito. Está no histórico"
        })
    }

    /// Regras de fala e dicionário sobre o texto de cada trecho de arquivo ou reunião.
    pub fn preparar(&self, texto: &str) -> String {
        let termos = self.b().termos().unwrap_or_default();
        let filtrado = RegrasFala::filtrar(texto);
        RegrasFala::arrumar_espacos(&AplicadorDicionario::new(termos, None).aplicar(&filtrado).0)
    }

    /// `ImportadorWispr` não existe no Windows (o Wispr Flow para Windows não usa banco equivalente conhecido).
    pub fn importar_wispr(&self) -> Res<Value> {
        Err("Importar do Wispr Flow não está disponível no Windows.".into())
    }
}

#[cfg(test)]
pub mod dubles {
    //! Motores de mentira: palavras a partir da energia do áudio, sem modelo nenhum.
    use std::{
        path::Path,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
    };

    use nucleo::motores::{
        Erro, FaixaDeVoz, IdentificadorDeFalantes, Limpador, PalavraReconhecida, PedidoLimpeza,
        ReconhecedorDeFala, Transcritor,
    };
    use serde_json::{json, Value};

    use super::{Fala, Motores};

    pub const TAXA: usize = 16_000;

    /// Tom de 440 Hz com volume 0,3 em `fala` segundos, depois `silencio` segundos de silêncio.
    pub fn voz(fala: f64, silencio: f64) -> Vec<f32> {
        let n = (fala * TAXA as f64) as usize;
        let mut v: Vec<f32> = (0..n)
            .map(|i| 0.3 * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / TAXA as f32).sin())
            .collect();
        v.extend(std::iter::repeat_n(0.0, (silencio * TAXA as f64) as usize));
        v
    }

    /// Uma palavra ("palavraK") por célula de 0,5 s com som, numa grade absoluta: a mesma fala vista
    /// por janelas diferentes dá a mesma palavra com o mesmo tempo. Célula cortada pela janela fica de fora.
    pub struct FalaDuble {
        pub chamadas: AtomicUsize,
    }

    impl Transcritor for FalaDuble {
        fn nome(&self) -> &str {
            "duble"
        }
        fn transcrever(&self, amostras: &[f32]) -> Result<String, Erro> {
            self.chamadas.fetch_add(1, Ordering::Relaxed);
            Ok(self
                .palavras(amostras, 0.0, None)?
                .into_iter()
                .map(|p| p.texto)
                .collect::<Vec<_>>()
                .join(" "))
        }
    }

    impl ReconhecedorDeFala for FalaDuble {
        fn palavras(
            &self,
            amostras: &[f32],
            inicio: f64,
            _fim: Option<f64>,
        ) -> Result<Vec<PalavraReconhecida>, Erro> {
            let fim = inicio + amostras.len() as f64 / TAXA as f64;
            let (primeira, ultima) = (
                (inicio * 2.0 - 1e-6).ceil() as i64,
                (fim * 2.0 + 1e-6).floor() as i64,
            );
            if primeira >= ultima {
                // Janela curta (bloco selado numa pausa), sem célula inteira: um reconhecedor real
                // ainda devolveria a fala que houver nela.
                return Ok(if amostras.iter().any(|x| x.abs() > 0.05) {
                    vec![PalavraReconhecida {
                        texto: format!("trecho{}", (inicio * 100.0).round() as i64),
                        inicio,
                        fim,
                    }]
                } else {
                    vec![]
                });
            }
            Ok((primeira..ultima)
                .filter_map(|k| {
                    let a = ((k as f64 * 0.5 - inicio) * TAXA as f64).round().max(0.0) as usize;
                    let b = (a + TAXA / 2).min(amostras.len());
                    amostras
                        .get(a..b)
                        .filter(|c| c.iter().any(|x| x.abs() > 0.05))
                        .map(|_| PalavraReconhecida {
                            texto: format!("palavra{k}"),
                            inicio: k as f64 * 0.5 + 0.05,
                            fim: k as f64 * 0.5 + 0.45,
                        })
                })
                .collect())
        }
    }

    pub struct FalantesDuble;
    impl IdentificadorDeFalantes for FalantesDuble {
        fn faixas(&self, wav: &Path) -> Result<Vec<FaixaDeVoz>, Erro> {
            let tamanho = std::fs::metadata(wav)?.len();
            Ok(vec![FaixaDeVoz {
                voz: "offline-1".into(),
                inicio: 0.0,
                fim: tamanho as f64 / 32_000.0 + 1.0,
            }])
        }
    }

    pub struct LimpadorDuble;
    impl Limpador for LimpadorDuble {
        fn nome(&self) -> &str {
            "limpador-duble"
        }
        fn limpar(&self, p: &PedidoLimpeza) -> Result<String, Erro> {
            Ok(p.transcricao.clone())
        }
    }

    /// Reconhecedor que sempre falha (para os caminhos de erro do processamento).
    pub struct FalaQuebrada;
    impl Transcritor for FalaQuebrada {
        fn nome(&self) -> &str {
            "quebrada"
        }
        fn transcrever(&self, _a: &[f32]) -> Result<String, Erro> {
            Err("reconhecedor quebrado".into())
        }
    }
    impl ReconhecedorDeFala for FalaQuebrada {
        fn palavras(
            &self,
            _a: &[f32],
            _i: f64,
            _f: Option<f64>,
        ) -> Result<Vec<PalavraReconhecida>, Erro> {
            Err("reconhecedor quebrado".into())
        }
    }

    pub struct MotoresDuble {
        pub fala: Arc<FalaDuble>,
        pub com_falantes: bool,
        pub com_limpeza: bool,
        /// O resumo espera até ser cancelado (no máximo 5 s).
        pub resumo_lento: bool,
        pub fala_quebrada: bool,
    }

    impl MotoresDuble {
        pub fn novo() -> Arc<Self> {
            Self::com(|_| {})
        }
        pub fn com(ajustar: impl FnOnce(&mut Self)) -> Arc<Self> {
            let mut m = Self {
                fala: Arc::new(FalaDuble {
                    chamadas: AtomicUsize::new(0),
                }),
                com_falantes: true,
                com_limpeza: false,
                resumo_lento: false,
                fala_quebrada: false,
            };
            ajustar(&mut m);
            Arc::new(m)
        }
    }

    impl Motores for MotoresDuble {
        fn fala(&self, _esperar: bool) -> Result<Arc<dyn Fala>, super::FalaIndisponivel> {
            Ok(if self.fala_quebrada {
                Arc::new(FalaQuebrada)
            } else {
                self.fala.clone()
            })
        }
        fn fala_disponivel(&self) -> bool {
            true
        }
        fn falantes(&self) -> Option<Arc<dyn IdentificadorDeFalantes>> {
            self.com_falantes
                .then(|| Arc::new(FalantesDuble) as Arc<dyn IdentificadorDeFalantes>)
        }
        fn limpador(&self) -> Option<Arc<dyn Limpador>> {
            self.com_limpeza
                .then(|| Arc::new(LimpadorDuble) as Arc<dyn Limpador>)
        }
        fn resumo_disponivel(&self) -> bool {
            self.com_limpeza
        }
        fn resumir(
            &self,
            linhas: &[String],
            cancelado: &dyn Fn() -> bool,
        ) -> Result<Vec<String>, String> {
            let inicio = std::time::Instant::now();
            while self.resumo_lento
                && !cancelado()
                && inicio.elapsed() < std::time::Duration::from_secs(5)
            {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            if cancelado() {
                return Err("cancelado".into());
            }
            Ok(vec![format!("{} falas", linhas.len())])
        }
        fn estado_reconhecedor(&self) -> (String, String) {
            ("pronto".into(), String::new())
        }
        fn estado_limpador(&self) -> (String, String) {
            ("desligado".into(), String::new())
        }
        fn modelo_ia(&self) -> Value {
            json!({"estado": "instalado", "arquivo": "duble.gguf", "tamanho": 1})
        }
        fn baixar_modelo(&self) -> Result<(), String> {
            Ok(())
        }
        fn cancelar_modelo(&self) {}
        fn llama_instalado(&self) -> bool {
            false
        }
    }
}

#[cfg(test)]
mod testes {
    use super::{dubles::*, *};

    fn integracao() -> (tempfile::TempDir, Integracao) {
        let tmp = crate::caminhos::pasta_de_teste();
        let i = Integracao::abrir(&tmp.path().join("t.sqlite"), MotoresDuble::novo()).unwrap();
        (tmp, i)
    }

    fn ditado(i: &Integracao) -> Ditado {
        let mut d = Ditado {
            iniciado_em: agora(),
            app: "notepad.exe".into(),
            app_nome: "notepad".into(),
            motor: "duble".into(),
            ..Default::default()
        };
        d.id = i.inserir_ditado(&d).unwrap();
        d
    }

    #[test]
    fn ditado_de_ponta_a_ponta_grava_historico_e_ultimo_texto() {
        let (_t, i) = integracao();
        let mut d = ditado(&i);
        let mut s = i.nova_sessao(d.id, false).unwrap();
        for bloco in voz(2.0, 1.5).chunks(1600) {
            s.alimentar(bloco);
        }
        let r = s.finalizar();
        assert!(r.texto.contains("palavra"), "{r:?}");
        assert_eq!(r.falhas, 0);
        d.estado = "concluido".into();
        d.duracao = 3.5;
        i.concluir_ditado(&d, &r).unwrap();
        let h = i.historico(10, "").unwrap();
        assert_eq!(h.as_array().unwrap().len(), 1);
        assert_eq!(h[0]["estado"], "concluido");
        assert_eq!(h[0]["appNome"], "notepad");
        assert_eq!(i.ultimo_texto().unwrap().unwrap(), r.texto);
        let e = i.estatisticas(30).unwrap();
        assert_eq!(e["totalDitados"], 1);
    }

    /// M6: com pasta de trabalho, o ditado funciona e a pasta é a do app.
    #[test]
    fn sessao_usa_a_pasta_de_trabalho_do_app() {
        let tmp = crate::caminhos::pasta_de_teste();
        let i = Integracao::abrir(&tmp.path().join("t.sqlite"), MotoresDuble::novo())
            .unwrap()
            .com_pasta_de_trabalho(tmp.path().join("Trabalho").join("ditado"));
        assert_eq!(
            i.pasta_de_trabalho.as_deref(),
            Some(tmp.path().join("Trabalho").join("ditado").as_path())
        );
        assert!(tmp.path().join("Trabalho").join("ditado").is_dir());
        let d = ditado(&i);
        let mut s = i.nova_sessao(d.id, false).unwrap();
        for bloco in voz(2.0, 1.0).chunks(1600) {
            s.alimentar(bloco);
        }
        assert!(s.finalizar().texto.contains("palavra"));
    }

    #[test]
    fn trechos_vao_para_a_reserva_e_voltam_na_recuperacao() {
        let tmp = crate::caminhos::pasta_de_teste();
        let banco = tmp.path().join("t.sqlite");
        {
            let i = Integracao::abrir(&banco, MotoresDuble::novo()).unwrap();
            let d = ditado(&i);
            let mut s = i.nova_sessao(d.id, false).unwrap();
            for bloco in voz(2.0, 1.5).chunks(1600) {
                s.alimentar(bloco);
            }
            // O app "cai": a sessão termina, mas o ditado nunca é concluído.
            let _ = s.finalizar();
        }
        let i = Integracao::abrir(&banco, MotoresDuble::novo()).unwrap();
        assert_eq!(i.recuperar_interrompidos(tmp.path()).unwrap(), 1);
        let h = i.historico(10, "").unwrap();
        assert_eq!(h[0]["estado"], "recuperado");
        assert!(h[0]["texto"].as_str().unwrap().contains("palavra"));
    }

    #[test]
    fn interrompido_sem_texto_com_audio_vira_falhou_e_sem_audio_sai() {
        let tmp = crate::caminhos::pasta_de_teste();
        let i = Integracao::abrir(&tmp.path().join("t.sqlite"), MotoresDuble::novo()).unwrap();
        let com = ditado(&i);
        i.definir_audio(com.id, "1.wav").unwrap();
        std::fs::write(tmp.path().join("1.wav"), b"x").unwrap();
        let _sem = ditado(&i);
        i.recuperar_interrompidos(tmp.path()).unwrap();
        let h = i.historico(10, "").unwrap();
        assert_eq!(h.as_array().unwrap().len(), 1);
        assert_eq!(h[0]["estado"], "falhou");
    }

    #[test]
    fn motivo_da_fala_indisponivel_e_o_texto_da_pilula() {
        assert_eq!(
            FalaIndisponivel::Baixando {
                feito: 40,
                total: 100
            }
            .to_string(),
            "Baixando o modelo de fala: 40%"
        );
        assert_eq!(
            FalaIndisponivel::Baixando { feito: 1, total: 0 }.to_string(),
            "Baixando o modelo de fala: 0%"
        );
        assert_eq!(FalaIndisponivel::Falhou("x".into()).to_string(), "x");
    }

    #[test]
    fn dicionario_salva_preserva_usos_e_apaga() {
        let (_t, i) = integracao();
        let t = i
            .salvar_termo(0, "Dita.se", vec!["dita se".into()])
            .unwrap();
        let id = t["id"].as_i64().unwrap();
        let t2 = i
            .salvar_termo(id, "Dita.se", vec!["ditasse".into()])
            .unwrap();
        assert_eq!(t2["ouvidoComo"][0], "ditasse");
        assert_eq!(i.termos().unwrap().as_array().unwrap().len(), 1);
        i.apagar_termo(id).unwrap();
        assert!(i.termos().unwrap().as_array().unwrap().is_empty());
    }

    #[test]
    fn retranscrever_usa_o_audio_guardado() {
        let tmp = crate::caminhos::pasta_de_teste();
        let i = Integracao::abrir(&tmp.path().join("t.sqlite"), MotoresDuble::novo()).unwrap();
        let mut d = ditado(&i);
        let mut w = crate::audio::EscritorWav::criar(&tmp.path().join("7.wav")).unwrap();
        w.gravar(&voz(2.0, 1.0)).unwrap();
        d.audio = "7.wav".into();
        d.estado = "falhou".into();
        i.definir_audio(d.id, "7.wav").unwrap();
        i.marcar_falha(&d).unwrap();
        assert_eq!(
            i.retranscrever(d.id, tmp.path(), false).unwrap(),
            "Transcrito. Está no histórico"
        );
        let h = i.historico(10, "").unwrap();
        assert_eq!(h[0]["estado"], "concluido");
    }
}
