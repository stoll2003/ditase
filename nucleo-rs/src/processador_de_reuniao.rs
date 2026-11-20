//! Transcreve os blocos de uma reunião pela costura de cada fonte e geração (Swift
//! `ProcessadorDeReuniao`). Ao vivo, na pausa, no fim e na recuperação o caminho é o mesmo; só muda
//! se a cauda é fechada. O estado está no banco: outra instância (ou uma nova abertura) continua de
//! onde a última confirmação parou.
use crate::processador_de_arquivo::recuperar_cauda;
use crate::{motores::*, *};
use std::collections::{HashMap, HashSet};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FalhaReuniao {
    /// Outra tentativa já está transcrevendo esta reunião.
    Concorrencia,
    /// Ainda há áudio sem transcrição nesta reunião.
    TextoIncompleto,
}

impl fmt::Display for FalhaReuniao {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Concorrencia => "Outra tentativa já está transcrevendo esta reunião.",
            Self::TextoIncompleto => "Ainda há áudio sem transcrição nesta reunião.",
        })
    }
}

impl std::error::Error for FalhaReuniao {}

/// Contexto antes do ponto confirmado, margem antes do fim disponível, passo mínimo e janela máxima.
pub const CONTEXTO: usize = TAXA as usize;
pub const MARGEM: usize = TAXA as usize * 8 / 10;
pub const PASSO_MINIMO: usize = TAXA as usize * 2;
pub const JANELA_MAXIMA: usize = TAXA as usize * 15;

const LIMITE_SOM_CURTO: usize = TAXA as usize * 3 / 10;
const MOTIVO_RETIDO: &str =
    "Um som breve ficou sem texto após duas tentativas. O áudio continua guardado.";
const MOTIVO_INCOMPLETO: &str = "Áudio do bloco ausente ou incompleto.";

struct TrechoParaDiarizar {
    id: String,
    inicio: f64,
    fim: f64,
}

pub struct ProcessadorDeReuniao<'a> {
    pub documento: &'a str,
    pub armazem: &'a mut ArmazemLancamento,
    pub disco: &'a DiscoDeBlocos,
    pub reconhecedor: &'a dyn ReconhecedorDeFala,
    preparar: Option<&'a dyn Fn(&str) -> String>,
    dono: String,
    tomadas: HashSet<(FonteDeAudio, u64)>,
}

fn novo_dono() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!(
        "{}-{}-{}",
        std::process::id(),
        t,
        N.fetch_add(1, Ordering::Relaxed)
    )
}

impl<'a> ProcessadorDeReuniao<'a> {
    pub fn new(
        documento: &'a str,
        armazem: &'a mut ArmazemLancamento,
        disco: &'a DiscoDeBlocos,
        reconhecedor: &'a dyn ReconhecedorDeFala,
    ) -> Self {
        Self {
            documento,
            armazem,
            disco,
            reconhecedor,
            preparar: None,
            dono: novo_dono(),
            tomadas: HashSet::new(),
        }
    }

    /// Preparação do texto de cada trecho antes de gravar (Swift `preparar`). Texto vazio mantém o original.
    pub fn com_preparar(mut self, preparar: &'a dyn Fn(&str) -> String) -> Self {
        self.preparar = Some(preparar);
        self
    }

    fn preparado(&self, texto: &str) -> String {
        match self.preparar {
            Some(p) => {
                let e = p(texto);
                if e.trim().is_empty() {
                    texto.to_string()
                } else {
                    e
                }
            }
            None => texto.to_string(),
        }
    }

    /// Há um bloco novo durável: processa o que já dá para decidir, sem fechar a cauda.
    /// Devolve os trechos com o ID com que ficaram no banco (o evento ao vivo).
    pub fn bloco_selado(&mut self) -> Result<Vec<TrechoNovo>, Erro> {
        self.avancar(false)
    }

    /// Fecha as caudas de todas as fontes (pausa, fim ou recuperação).
    pub fn fechar(&mut self) -> Result<Vec<TrechoNovo>, Erro> {
        let mut trechos = self.avancar(true)?;
        trechos.extend(self.tentar_pendencias(true)?);
        Ok(trechos)
    }

    /// Recuperar é transcrever: fecha as costuras abertas e tenta de novo cada faixa pendente.
    /// A adoção de PCM órfão é feita antes, por `ArmazemLancamento::preparar_abertura`.
    pub fn recuperar(&mut self) -> Result<Vec<TrechoNovo>, Erro> {
        // Versões anteriores podiam fechar a costura mesmo com blocos ainda não reconhecidos.
        self.armazem.db.execute(
            "UPDATE lt_costuras AS c SET fechada=0 WHERE c.documento=? AND c.fechada=1
             AND EXISTS (SELECT 1 FROM lt_blocos b WHERE b.documento=c.documento
                AND b.fonte=c.fonte AND b.geracao=c.geracao AND b.estado='capturado'
                AND b.inicio+b.quantidade>c.confirmado_ate)",
            [self.documento],
        )?;
        let mut t = self.avancar(true)?;
        t.extend(self.repetir_pendencias()?);
        Ok(t)
    }

    fn avancar(&mut self, fechamento: bool) -> Result<Vec<TrechoNovo>, Erro> {
        let mut persistidos = vec![];
        for costura in self.armazem.costuras(self.documento)? {
            if costura.fechada {
                continue;
            }
            let (f, g) = (costura.fonte, costura.geracao);
            let disponivel: Option<usize> = self.armazem.db.prepare_cached(
                "SELECT MAX(inicio+quantidade) FROM lt_blocos WHERE documento=? AND fonte=? AND geracao=?",
            )?.query_row(rusqlite::params![self.documento, f.nome(), g], |r| r.get(0))?;
            let Some(disponivel) = disponivel else {
                continue;
            };
            let mut confirmado = costura.confirmado_ate;
            loop {
                let fim_janela = disponivel.min(confirmado.saturating_add(JANELA_MAXIMA));
                let fecha = fechamento && fim_janela == disponivel;
                let limite = if fecha {
                    fim_janela
                } else {
                    fim_janela.saturating_sub(MARGEM)
                };
                if limite <= confirmado || (!fecha && limite - confirmado < PASSO_MINIMO) {
                    break;
                }
                if !self.tomadas.contains(&(f, g)) {
                    self.armazem
                        .tomar_costura(self.documento, f, g, &self.dono)?;
                    self.tomadas.insert((f, g));
                }
                let (novo, trechos) = self.passo(f, g, confirmado, fim_janela, limite, fecha)?;
                persistidos.extend(trechos);
                if novo == confirmado {
                    break;
                }
                confirmado = novo;
            }
        }
        Ok(persistidos)
    }

    /// Um passo: lê a janela com contexto, reconhece, decide o corte e confirma numa transação.
    fn passo(
        &mut self,
        f: FonteDeAudio,
        g: u64,
        confirmado: usize,
        fim_janela: usize,
        limite: usize,
        fechamento: bool,
    ) -> Result<(usize, Vec<TrechoNovo>), Erro> {
        let taxa = f64::from(TAXA);
        let inicio_geracao: usize = self
            .armazem
            .db
            .prepare_cached(
                "SELECT MIN(inicio) FROM lt_blocos WHERE documento=? AND fonte=? AND geracao=?",
            )?
            .query_row(rusqlite::params![self.documento, f.nome(), g], |r| r.get(0))?;
        let de = inicio_geracao.max(confirmado.saturating_sub(CONTEXTO));
        let blocos = self
            .armazem
            .blocos_na_janela(self.documento, f, g, de, fim_janela)?;
        let refs: Vec<_> = blocos.iter().collect();
        let (audio, quebrados) = self.disco.faixa(&refs, de, fim_janela);
        for b in &quebrados {
            self.armazem.marcar_lacuna(&b.id, MOTIVO_INCOMPLETO)?;
        }
        let mut palavras = if AtividadeDeVoz::tem(&audio) {
            self.reconhecedor.palavras(
                &audio,
                de as f64 / taxa,
                fechamento.then_some((fim_janela - de) as f64 / taxa),
            )?
        } else {
            vec![]
        };
        let mut cauda_pendente = None;
        if fechamento {
            let mut inicio_cauda = fim_janela - audio.len().min(CaudaDaFala::CAPACIDADE);
            let mut trecho_final = &audio[audio.len() - (fim_janela - inicio_cauda)..];
            let audio_expandido;
            let ultimo_fim = palavras.last().map(|p| p.fim);
            let corte_atual = (confirmado as f64 / taxa).max(ultimo_fim.unwrap_or(0.));
            let mut recuperada = recuperar_cauda(
                trecho_final,
                inicio_cauda as f64 / taxa,
                corte_atual,
                self.reconhecedor,
            )?;
            if recuperada.is_none() && fim_janela as f64 / taxa - corte_atual >= 0.28 {
                let expandido =
                    inicio_geracao.max(fim_janela.saturating_sub(CaudaDaFala::CAPACIDADE));
                if expandido < de {
                    let contexto = self.armazem.blocos_na_janela(
                        self.documento,
                        f,
                        g,
                        expandido,
                        fim_janela,
                    )?;
                    let refs: Vec<_> = contexto.iter().collect();
                    let (pcm, quebrados) = self.disco.faixa(&refs, expandido, fim_janela);
                    for b in quebrados {
                        self.armazem.marcar_lacuna(&b.id, MOTIVO_INCOMPLETO)?;
                    }
                    audio_expandido = pcm;
                    trecho_final = &audio_expandido;
                    inicio_cauda = expandido;
                    recuperada = recuperar_cauda(
                        trecho_final,
                        inicio_cauda as f64 / taxa,
                        corte_atual,
                        self.reconhecedor,
                    )?;
                }
            }
            if let Some((lim, extra)) = recuperada {
                palavras.retain(|p| p.inicio < lim);
                palavras.extend(extra);
            } else if let (Some(fala), Some(u)) = (CaudaDaFala::localizar(trecho_final), ultimo_fim)
            {
                if inicio_cauda + fala.inicio >= confirmado.max(((u + 0.16) * taxa) as usize) {
                    cauda_pendente = Some(inicio_cauda + fala.inicio..inicio_cauda + fala.fim);
                }
            }
        }
        let atividade = |r: std::ops::Range<usize>| {
            let a = r.start.saturating_sub(de);
            let z = audio.len().min(r.end.saturating_sub(de));
            a < z && AtividadeDeVoz::tem(&audio[a..z])
        };
        let decisao = Costura::decidir(&palavras, confirmado, limite, fechamento, atividade);
        if decisao.novo_confirmado <= confirmado {
            return Ok((confirmado, vec![]));
        }
        let trechos: Vec<TrechoNovo> =
            Costura::trechos(&decisao.confirmadas, self.documento, f, g, None)
                .into_iter()
                .map(|t| TrechoNovo {
                    texto: self.preparado(&t.texto),
                    voz: f.voz_ao_vivo().into(),
                    ..t
                })
                .collect();
        let com_atividade = blocos
            .iter()
            .filter(|b| {
                b.estado == EstadoBloco::Capturado
                    && b.fim() <= decisao.novo_confirmado
                    && atividade(b.inicio..b.fim())
            })
            .map(|b| b.id.texto())
            .collect();
        // Uma palavra na margem pode justificar um corte sem palavra antes dele.
        // Se havia atividade nessa parte, mantenha a faixa para nova tentativa.
        let sem_palavras = confirmado..decisao.novo_confirmado;
        let sem_texto = decisao.confirmadas.is_empty() && atividade(sem_palavras.clone());
        let pendencia = decisao
            .pendente
            .or(cauda_pendente)
            .or(sem_texto.then_some(sem_palavras))
            .map(|r| {
                (
                    r.start,
                    r.end,
                    "Havia voz, mas o reconhecedor não devolveu texto. O áudio continua guardado."
                        .to_string(),
                )
            });
        let passo = Passo {
            documento: self.documento.into(),
            fonte: f,
            geracao: g,
            dono: self.dono.clone(),
            esperado: confirmado,
            novo: decisao.novo_confirmado,
            trechos,
            com_atividade,
            pendencia,
            fechar: false,
        };
        let persistidos = self
            .armazem
            .confirmar_persistindo(&passo)?
            .ok_or(FalhaReuniao::Concorrencia)?;
        Ok((decisao.novo_confirmado, persistidos))
    }

    fn trechos_para_diarizar(&self) -> Result<Vec<TrechoParaDiarizar>, Erro> {
        let mut s = self.armazem.db.prepare_cached(
            "SELECT id,inicio,fim FROM lt_trechos WHERE documento=? AND versao='' AND fonte='mac'
             AND voz_final IS NULL AND (voz='chamada' OR voz GLOB 'fluxo-*')",
        )?;
        let linhas = s.query_map([self.documento], |r| {
            Ok(TrechoParaDiarizar {
                id: r.get(0)?,
                inicio: r.get(1)?,
                fim: r.get(2)?,
            })
        })?;
        Ok(linhas.collect::<rusqlite::Result<_>>()?)
    }

    /// Nova tentativa de cada faixa sem texto, inclusive as retidas após a conclusão.
    pub fn repetir_pendencias(&mut self) -> Result<Vec<TrechoNovo>, Erro> {
        self.tentar_pendencias(false)
    }

    fn tentar_pendencias(&mut self, so_curtas: bool) -> Result<Vec<TrechoNovo>, Erro> {
        let mut persistidos = vec![];
        for p in self.armazem.pendencias(self.documento)? {
            if p.estado != "aberta" && (so_curtas || p.estado != "retida") {
                continue;
            }
            if so_curtas && p.id.starts_with("adocao|") {
                continue;
            }
            if let Some(desfecho) = self.tentar_pendencia(&p, so_curtas)? {
                persistidos.extend(
                    self.armazem
                        .resolver_persistindo(&p.id, self.documento, p.tentativas, desfecho)?
                        .ok_or(FalhaReuniao::Concorrencia)?,
                );
            }
        }
        Ok(persistidos)
    }

    fn tentar_pendencia(
        &mut self,
        p: &Pendencia,
        so_curtas: bool,
    ) -> Result<Option<DesfechoPendencia>, Erro> {
        let taxa = f64::from(TAXA);
        let adocao = p.id.starts_with("adocao|");
        let (pi, pf) = (p.inicio as f64 / taxa, p.fim as f64 / taxa);
        // Intervalo com texto já confirmado: preservar a faixa sem deduzir o que falta.
        let sobre_texto = self
            .armazem
            .db
            .prepare_cached(
                "SELECT 1 FROM lt_trechos WHERE documento=?1 AND versao='' AND fonte=?2
             AND inicio<?4 AND fim>?3
             AND (?5 OR ((inicio+fim)/2>=?3 AND (inicio+fim)/2<?4) OR (inicio<=?3 AND fim>=?4))
             LIMIT 1",
            )?
            .exists(rusqlite::params![
                self.documento,
                p.fonte.nome(),
                pi,
                pf,
                adocao
            ])?;
        if sobre_texto {
            return Ok((!so_curtas).then(|| DesfechoPendencia::Continua(
                "Áudio guardado cobre um trecho que já tem texto. Para não duplicar nem perder fala, ele continua guardado até você decidir.".into()
            )));
        }

        let janela_inicio = p.inicio.saturating_sub(CONTEXTO);
        let janela_fim = p.fim.saturating_add(CONTEXTO);
        let blocos = self.armazem.blocos_na_janela(
            self.documento,
            p.fonte,
            p.geracao,
            janela_inicio,
            janela_fim,
        )?;
        let de = blocos
            .iter()
            .map(|b| b.inicio)
            .min()
            .unwrap_or(p.inicio)
            .max(janela_inicio)
            .min(p.inicio);
        let ate = blocos
            .iter()
            .map(|b| b.fim())
            .max()
            .unwrap_or(p.fim)
            .min(janela_fim)
            .max(p.fim);
        let refs: Vec<_> = blocos.iter().collect();
        let (audio, quebrados) = self.disco.faixa(&refs, de, ate);
        for b in &quebrados {
            self.armazem.marcar_lacuna(&b.id, MOTIVO_INCOMPLETO)?;
        }
        let completo = quebrados
            .iter()
            .all(|b| b.inicio >= p.fim || b.fim() <= p.inicio)
            && cobre_intervalo(&blocos, p.inicio, p.fim);
        let trecho = &audio[p.inicio - de..p.fim - de];
        let curto = !adocao && completo && AtividadeDeVoz::som_curto(trecho, LIMITE_SOM_CURTO);
        if so_curtas && !curto {
            return Ok(None);
        }
        let palavras: Vec<_> = self
            .reconhecedor
            .palavras(&audio, de as f64 / taxa, Some((ate - de) as f64 / taxa))?
            .into_iter()
            .filter(|w| {
                let c = (w.inicio + w.fim) / 2.;
                c >= pi && c < pf
            })
            .collect();
        let desfecho = if palavras.is_empty() {
            if !completo {
                DesfechoPendencia::Continua(
                    "Parte do áudio foi perdida. Confira a lacuna na transcrição.".into(),
                )
            } else if curto {
                DesfechoPendencia::Retida(MOTIVO_RETIDO.into())
            } else if AtividadeDeVoz::tem(trecho) {
                DesfechoPendencia::Continua(format!(
                    "Havia voz, mas o reconhecedor não devolveu texto em {} tentativas. O áudio continua guardado.",
                    p.tentativas + 1
                ))
            } else {
                DesfechoPendencia::SemFala
            }
        } else {
            DesfechoPendencia::Resolvida(
                Costura::trechos(&palavras, self.documento, p.fonte, p.geracao, None)
                    .into_iter()
                    .map(|t| TrechoNovo {
                        texto: self.preparado(&t.texto),
                        ..t
                    })
                    .collect(),
            )
        };
        Ok(Some(desfecho))
    }

    /// Fecha costuras no fim da reunião. Uma costura incompleta ainda pode ser recuperada.
    pub fn encerrar_costuras(&mut self) -> Result<(), Erro> {
        self.armazem.db.execute(
            "UPDATE lt_costuras AS c SET fechada=1 WHERE c.documento=? AND c.fechada=0
             AND NOT EXISTS (SELECT 1 FROM lt_blocos b WHERE b.documento=c.documento
                AND b.fonte=c.fonte AND b.geracao=c.geracao AND b.inicio+b.quantidade>c.confirmado_ate)",
            [self.documento],
        )?;
        Ok(())
    }

    /// Identifica as pessoas da chamada sobre o áudio retido e grava a atribuição por ID de trecho.
    /// Só depois do commit o áudio resolvido sai. Identificador ausente ou com falha devolve `false`
    /// e mantém o áudio para nova tentativa. Texto incompleto devolve erro.
    pub fn passada_final(
        &mut self,
        identificador: Option<&dyn IdentificadorDeFalantes>,
    ) -> Result<bool, Erro> {
        let mut blocos = self.armazem.blocos(self.documento)?;
        for b in blocos.iter_mut() {
            if b.arquivo.is_some()
                && !b.audio_liberado
                && b.estado != EstadoBloco::Lacuna
                && self.disco.ler_bytes(b).is_err()
            {
                self.armazem.marcar_lacuna(&b.id, MOTIVO_INCOMPLETO)?;
                b.estado = EstadoBloco::Lacuna;
            }
        }
        if blocos
            .iter()
            .any(|b| matches!(b.estado, EstadoBloco::Capturado | EstadoBloco::Pendente))
            || self
                .armazem
                .pendencias(self.documento)?
                .iter()
                .any(|p| p.estado == "aberta")
        {
            return Err(FalhaReuniao::TextoIncompleto.into());
        }
        let trechos = self.trechos_para_diarizar()?;
        let mut atribuicao = HashMap::new();
        let mut identificados = true;
        if !trechos.is_empty() {
            match identificador {
                Some(id) => {
                    let wav = self.disco.pasta(self.documento)?.join(format!(
                        "{}{}.wav",
                        disco_de_blocos::PREFIXO_WAV,
                        novo_dono()
                    ));
                    let mac: Vec<&BlocoRegistrado> = blocos
                        .iter()
                        .filter(|b| b.id.fonte == FonteDeAudio::Mac)
                        .collect();
                    let faixas = self
                        .disco
                        .escrever_wav(&mac, &wav)
                        .map_err(Erro::from)
                        .and_then(|_| id.faixas(&wav));
                    let _ = std::fs::remove_file(&wav);
                    match faixas {
                        Ok(faixas) => {
                            for t in trechos {
                                atribuicao.insert(
                                    t.id,
                                    AlinhamentoDeVoz::identificar(t.inicio, t.fim, &faixas),
                                );
                            }
                        }
                        Err(_) => identificados = false,
                    }
                }
                None => identificados = false,
            }
        }
        if !identificados {
            atribuicao.clear();
        }
        if !self.armazem.gravar_falantes(
            self.documento,
            &atribuicao,
            identificados,
            identificados,
        )? {
            return Err(FalhaReuniao::TextoIncompleto.into());
        }
        if identificados {
            self.limpar_arquivos();
        }
        Ok(identificados)
    }

    /// Remove os arquivos já liberados no banco desta reunião. Repetida na abertura se interrompida.
    pub fn limpar_arquivos(&mut self) {
        let Ok(lista) = self.armazem.arquivos_para_limpar() else {
            return;
        };
        for (id, arquivo) in lista {
            if id.documento != self.documento {
                continue;
            }
            // Referência que não é bloco desta reunião não é seguida: só ela sai do banco.
            if DiscoDeBlocos::arquivo_valido(&arquivo, self.documento)
                && self.disco.apagar(self.documento, &arquivo).is_err()
            {
                continue;
            }
            let _ = self.armazem.esquecer_arquivo(&id);
        }
    }
}

/// Confere cobertura contínua. Silêncio preenchido pela leitura de uma janela pode ser uma lacuna.
fn cobre_intervalo(blocos: &[BlocoRegistrado], inicio: usize, fim: usize) -> bool {
    let mut validos: Vec<_> = blocos
        .iter()
        .filter(|b| {
            !b.audio_liberado
                && b.arquivo.is_some()
                && b.estado != EstadoBloco::Lacuna
                && b.estado != EstadoBloco::Descartado
        })
        .collect();
    validos.sort_unstable_by_key(|b| b.inicio);
    let mut ate = inicio;
    for b in validos {
        if b.inicio > ate || ate >= fim {
            break;
        }
        ate = ate.max(b.fim());
    }
    ate >= fim
}
