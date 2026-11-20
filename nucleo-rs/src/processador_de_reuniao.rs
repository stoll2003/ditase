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

const MOTIVO_INCOMPLETO: &str = "Áudio do bloco ausente ou incompleto.";

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
        self.avancar(true)
    }

    /// Recuperar é transcrever: fecha as costuras abertas e tenta de novo cada faixa pendente.
    /// A adoção de PCM órfão é feita antes, por `ArmazemLancamento::preparar_abertura`.
    pub fn recuperar(&mut self) -> Result<Vec<TrechoNovo>, Erro> {
        let mut t = self.fechar()?;
        t.extend(self.repetir_pendencias()?);
        Ok(t)
    }

    fn avancar(&mut self, fechamento: bool) -> Result<Vec<TrechoNovo>, Erro> {
        let mut persistidos = vec![];
        let blocos = self.armazem.blocos(self.documento)?;
        for costura in self.armazem.costuras(self.documento)? {
            if costura.fechada {
                continue;
            }
            let (f, g) = (costura.fonte, costura.geracao);
            let Some(disponivel) = blocos
                .iter()
                .filter(|b| b.id.fonte == f && b.id.geracao == g)
                .map(|b| b.fim())
                .max()
            else {
                continue;
            };
            if self.tomadas.insert((f, g)) {
                self.armazem
                    .tomar_costura(self.documento, f, g, &self.dono)?;
            }
            let mut confirmado = costura.confirmado_ate;
            loop {
                let fim_janela = disponivel.min(confirmado + JANELA_MAXIMA);
                let fecha = fechamento && fim_janela == disponivel;
                let limite = if fecha {
                    fim_janela
                } else {
                    fim_janela.saturating_sub(MARGEM)
                };
                if limite <= confirmado || (!fecha && limite - confirmado < PASSO_MINIMO) {
                    break;
                }
                let atual: Vec<_> = self
                    .armazem
                    .blocos(self.documento)?
                    .into_iter()
                    .filter(|b| b.id.fonte == f && b.id.geracao == g)
                    .collect();
                let (novo, trechos) =
                    self.passo(f, g, &atual, confirmado, fim_janela, limite, fecha)?;
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
    #[allow(clippy::too_many_arguments)]
    fn passo(
        &mut self,
        f: FonteDeAudio,
        g: u64,
        blocos: &[BlocoRegistrado],
        confirmado: usize,
        fim_janela: usize,
        limite: usize,
        fechamento: bool,
    ) -> Result<(usize, Vec<TrechoNovo>), Erro> {
        let taxa = f64::from(TAXA);
        let refs: Vec<&BlocoRegistrado> = blocos.iter().collect();
        let inicio_geracao = blocos.iter().map(|b| b.inicio).min().unwrap_or(confirmado);
        let de = inicio_geracao.max(confirmado.saturating_sub(CONTEXTO));
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
            let mut trecho_final = audio[audio.len() - (fim_janela - inicio_cauda)..].to_vec();
            let ultimo_fim = palavras.last().map(|p| p.fim);
            let corte_atual = (confirmado as f64 / taxa).max(ultimo_fim.unwrap_or(0.));
            let mut recuperada = recuperar_cauda(
                &trecho_final,
                inicio_cauda as f64 / taxa,
                corte_atual,
                self.reconhecedor,
            )?;
            if recuperada.is_none() && fim_janela as f64 / taxa - corte_atual >= 0.28 {
                let expandido =
                    inicio_geracao.max(fim_janela.saturating_sub(CaudaDaFala::CAPACIDADE));
                if expandido < de {
                    trecho_final = self.disco.faixa(&refs, expandido, fim_janela).0;
                    inicio_cauda = expandido;
                    recuperada = recuperar_cauda(
                        &trecho_final,
                        inicio_cauda as f64 / taxa,
                        corte_atual,
                        self.reconhecedor,
                    )?;
                }
            }
            if let Some((lim, extra)) = recuperada {
                palavras.retain(|p| p.inicio < lim);
                palavras.extend(extra);
            } else if let (Some(fala), Some(u)) =
                (CaudaDaFala::localizar(&trecho_final), ultimo_fim)
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
        let pendencia = decisao.pendente.or(cauda_pendente).map(|r| {
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

    /// Trechos crus da versão de reunião (sem edições): id, fonte, início, fim e voz.
    #[allow(clippy::type_complexity)]
    fn brutos(&self) -> Result<Vec<(String, FonteDeAudio, f64, f64, String)>, Erro> {
        let mut s = self.armazem.db.prepare(
            "SELECT id,fonte,inicio,fim,voz FROM lt_trechos WHERE documento=? AND versao=''",
        )?;
        let linhas = s.query_map([self.documento], |r| {
            Ok((
                r.get(0)?,
                disco_de_blocos::fonte(&r.get::<_, String>(1)?).unwrap_or(FonteDeAudio::Mac),
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
            ))
        })?;
        Ok(linhas.collect::<rusqlite::Result<_>>()?)
    }

    /// Nova tentativa de cada faixa com voz sem texto. Vazio de novo: continua pendente e repetível.
    pub fn repetir_pendencias(&mut self) -> Result<Vec<TrechoNovo>, Erro> {
        let taxa = f64::from(TAXA);
        let mut persistidos = vec![];
        let blocos = self.armazem.blocos(self.documento)?;
        for p in self.armazem.pendencias(self.documento)? {
            if p.estado != "aberta" {
                continue;
            }
            let da_fonte: Vec<&BlocoRegistrado> = blocos
                .iter()
                .filter(|b| b.id.fonte == p.fonte && b.id.geracao == p.geracao)
                .collect();
            let de = da_fonte
                .iter()
                .map(|b| b.inicio)
                .min()
                .unwrap_or(p.inicio)
                .max(p.inicio.saturating_sub(CONTEXTO));
            let ate = da_fonte
                .iter()
                .map(|b| b.fim())
                .max()
                .unwrap_or(p.fim)
                .min(p.fim + CONTEXTO);
            let (audio, quebrados) = self.disco.faixa(&da_fonte, de, ate);
            for b in &quebrados {
                self.armazem.marcar_lacuna(&b.id, MOTIVO_INCOMPLETO)?;
            }
            let sem_cobertura = quebrados
                .iter()
                .any(|b| b.inicio < p.fim && b.fim() > p.inicio)
                || da_fonte.iter().any(|b| {
                    b.estado == EstadoBloco::Lacuna && b.inicio < p.fim && b.fim() > p.inicio
                });
            // Pendência sobre intervalo que já tem texto da mesma fonte: não há como saber sem adivinhar
            // o que falta. Decide antes de reconhecer; o áudio fica guardado e o motivo aparece. Áudio
            // adotado retém em qualquer sobreposição; pendência normal só com sobreposição de verdade.
            let (pi, pf) = (p.inicio as f64 / taxa, p.fim as f64 / taxa);
            let adocao = p.id.starts_with("adocao|");
            let sobre_texto = self.brutos()?.iter().any(|(_, fonte, i, z, _)| {
                if *fonte != p.fonte || *i >= pf || *z <= pi {
                    return false;
                }
                let centro = (i + z) / 2.;
                adocao || (centro >= pi && centro < pf) || (*i <= pi && *z >= pf)
            });
            if sobre_texto {
                let motivo = "Áudio guardado cobre um trecho que já tem texto. Para não duplicar nem perder fala, ele continua guardado até você decidir.";
                if !self.armazem.resolver(
                    &p.id,
                    self.documento,
                    p.tentativas,
                    DesfechoPendencia::Continua(motivo.into()),
                )? {
                    return Err(FalhaReuniao::Concorrencia.into());
                }
                continue;
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
                let a = p.inicio.saturating_sub(de);
                let z = audio.len().min(p.fim.saturating_sub(de));
                if sem_cobertura {
                    DesfechoPendencia::Continua(
                        "Parte do áudio foi perdida. Confira a lacuna na transcrição.".into(),
                    )
                } else if a < z && AtividadeDeVoz::tem(&audio[a..z]) {
                    DesfechoPendencia::Continua(format!(
                        "Havia voz, mas o reconhecedor não devolveu texto em {} tentativas. O áudio continua guardado.",
                        p.tentativas + 1
                    ))
                } else {
                    DesfechoPendencia::SemFala
                }
            } else {
                let trechos: Vec<TrechoNovo> =
                    Costura::trechos(&palavras, self.documento, p.fonte, p.geracao, None)
                        .into_iter()
                        .map(|t| TrechoNovo {
                            texto: self.preparado(&t.texto),
                            voz: p.fonte.voz_ao_vivo().into(),
                            ..t
                        })
                        .collect();
                persistidos.extend(trechos.iter().cloned());
                DesfechoPendencia::Resolvida(trechos)
            };
            if !self
                .armazem
                .resolver(&p.id, self.documento, p.tentativas, desfecho)?
            {
                return Err(FalhaReuniao::Concorrencia.into());
            }
        }
        Ok(persistidos)
    }

    /// Fecha as costuras de vez no fim da reunião (nenhuma geração recebe mais áudio).
    pub fn encerrar_costuras(&mut self) -> Result<(), Erro> {
        for c in self.armazem.costuras(self.documento)? {
            if !c.fechada {
                self.armazem.db.execute(
                    "UPDATE lt_costuras SET fechada=1 WHERE documento=? AND fonte=? AND geracao=? AND confirmado_ate>=?",
                    rusqlite::params![self.documento, c.fonte.nome(), c.geracao, c.confirmado_ate],
                )?;
            }
        }
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
                && self.disco.ler(b).is_err()
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
        let da_chamada = self.armazem.documento(self.documento)?.map(|d| {
            d.trechos
                .iter()
                .any(|t| t.origem.as_deref() != Some("microfone"))
        });
        let mut atribuicao = HashMap::new();
        let mut identificados = true;
        if da_chamada == Some(true) {
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
                            // Só rótulos provisórios: "chamada" ao vivo e "fluxo-N" de diários antigos.
                            for (tid, fonte, i, z, voz) in self.brutos()? {
                                if fonte == FonteDeAudio::Mac
                                    && (voz == "chamada" || voz.starts_with("fluxo-"))
                                {
                                    atribuicao
                                        .insert(tid, AlinhamentoDeVoz::identificar(i, z, &faixas));
                                }
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
