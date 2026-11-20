//! Parte do `ArmazemLancamento` que espelha o restante da API Swift: estados e campos do documento,
//! costuras, confirmação de passo, pendências, versões, retenção de arquivos e a reconciliação de
//! abertura (`prepararAbertura` sem a migração do formato antigo do Mac).
use crate::armazem_lancamento::{agora, inserir};
use crate::disco_de_blocos::fonte;
use crate::{motores::Erro, *};
use rusqlite::{
    params, params_from_iter, Connection, OptionalExtension, Result as SqlResult,
    TransactionBehavior,
};
use std::collections::HashSet;

impl BlocoRegistrado {
    /// Amostra seguinte à última do bloco.
    pub fn fim(&self) -> usize {
        self.inicio + self.quantidade
    }
}

/// Campos editáveis do documento (Swift `CampoDocumento`).
#[derive(Debug, Clone, PartialEq)]
pub enum CampoDocumento {
    Nome(String),
    Oculto(bool),
    ErroCaptura(Option<String>),
    AvisoCaptura(Option<String>),
    Aviso(Option<String>),
    Duracao(f64),
    Atraso(f64),
    Resumo(Vec<String>),
    ErroResumo(String),
    ResumoCancelado,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EstadoCostura {
    pub fonte: FonteDeAudio,
    pub geracao: u64,
    pub confirmado_ate: usize,
    pub fechada: bool,
}

/// Um passo da costura confirmado numa só transação.
#[derive(Debug, Clone)]
pub struct Passo {
    pub documento: String,
    pub fonte: FonteDeAudio,
    pub geracao: u64,
    pub dono: String,
    pub esperado: usize,
    pub novo: usize,
    pub trechos: Vec<TrechoNovo>,
    /// Blocos com atividade de voz entre os que esta confirmação resolve.
    pub com_atividade: HashSet<String>,
    /// Início, fim e motivo de uma faixa com voz sem texto.
    pub pendencia: Option<(usize, usize, String)>,
    pub fechar: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pendencia {
    pub id: String,
    pub fonte: FonteDeAudio,
    pub geracao: u64,
    pub inicio: usize,
    pub fim: usize,
    pub motivo: String,
    pub tentativas: u32,
    pub estado: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DesfechoPendencia {
    Resolvida(Vec<TrechoNovo>),
    SemFala,
    Continua(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersaoDetalhada {
    pub id: String,
    pub documento: String,
    pub tentativa: i64,
    pub origem: String,
    pub estado: String,
    pub identidade: Option<String>,
}

/// Resultado da reconciliação de abertura (Swift `RelatorioMigracao`, sem os campos do formato antigo).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RelatorioAbertura {
    pub adotados: usize,
    pub lacunas: usize,
    pub invalidos: Vec<String>,
    pub falhas: Vec<String>,
}

const MOTIVO_INCOMPLETO: &str = "Áudio do bloco ausente ou incompleto.";

fn existe(c: &Connection, sql: &str, p: impl rusqlite::Params) -> SqlResult<bool> {
    Ok(c.query_row(sql, p, |_| Ok(())).optional()?.is_some())
}

fn pendencias_em(c: &Connection, doc: &str) -> SqlResult<Vec<Pendencia>> {
    let mut s = c.prepare(
        "SELECT id,fonte,geracao,inicio,fim,motivo,tentativas,estado FROM lt_pendencias WHERE documento=? ORDER BY inicio",
    )?;
    let linhas = s.query_map([doc], |r| {
        Ok(Pendencia {
            id: r.get(0)?,
            fonte: fonte(&r.get::<_, String>(1)?).unwrap_or(FonteDeAudio::Mac),
            geracao: r.get(2)?,
            inicio: r.get(3)?,
            fim: r.get(4)?,
            motivo: r.get(5)?,
            tentativas: r.get(6)?,
            estado: r.get(7)?,
        })
    })?;
    linhas.collect()
}

/// Bloco que sai de "pendente" quando nenhuma pendência aberta o cruza mais.
fn liberar_pendentes(
    c: &Connection,
    doc: &str,
    f: FonteDeAudio,
    g: u64,
    inicio: usize,
    fim: usize,
) -> SqlResult<()> {
    c.execute(
        "UPDATE lt_blocos SET estado='transcrito',erro=NULL WHERE documento=?1 AND fonte=?2 AND geracao=?3 AND estado='pendente' \
         AND inicio<?4 AND inicio+quantidade>?5 AND NOT EXISTS (SELECT 1 FROM lt_pendencias q WHERE q.documento=lt_blocos.documento \
         AND q.fonte=lt_blocos.fonte AND q.geracao=lt_blocos.geracao AND q.estado='aberta' AND q.inicio<lt_blocos.inicio+lt_blocos.quantidade \
         AND q.fim>lt_blocos.inicio)",
        params![doc, f.nome(), g, fim, inicio],
    )?;
    Ok(())
}

impl ArmazemLancamento {
    /// Muda o estado só se o atual está na lista esperada.
    pub fn mudar_estado(&self, doc: &str, para: &str, se: &[&str]) -> SqlResult<bool> {
        if se.is_empty() {
            return Ok(false);
        }
        let marcas = vec!["?"; se.len()].join(",");
        let sql = format!("UPDATE lt_documentos SET estado=? WHERE id=? AND estado IN ({marcas})");
        let mut valores: Vec<&str> = vec![para, doc];
        valores.extend_from_slice(se);
        Ok(self.db.execute(&sql, params_from_iter(valores))? == 1)
    }

    pub fn existe_documento(&self, id: &str) -> SqlResult<bool> {
        existe(&self.db, "SELECT 1 FROM lt_documentos WHERE id=?", [id])
    }

    pub fn definir(&self, doc: &str, campo: CampoDocumento) -> Result<(), Erro> {
        let c = &self.db;
        match campo {
            CampoDocumento::Nome(n) => c.execute("UPDATE lt_documentos SET nome=? WHERE id=?", params![n, doc])?,
            CampoDocumento::Oculto(o) => c.execute("UPDATE lt_documentos SET oculto=? WHERE id=?", params![o, doc])?,
            CampoDocumento::ErroCaptura(e) => c.execute("UPDATE lt_documentos SET erro_captura=? WHERE id=?", params![e, doc])?,
            CampoDocumento::AvisoCaptura(a) => c.execute("UPDATE lt_documentos SET aviso_captura=? WHERE id=?", params![a, doc])?,
            CampoDocumento::Aviso(a) => c.execute("UPDATE lt_documentos SET aviso=? WHERE id=?", params![a, doc])?,
            CampoDocumento::Duracao(d) => c.execute("UPDATE lt_documentos SET duracao=MAX(duracao,?) WHERE id=?", params![d, doc])?,
            CampoDocumento::Atraso(a) => c.execute("UPDATE lt_documentos SET atraso=? WHERE id=?", params![a, doc])?,
            CampoDocumento::Resumo(r) => c.execute(
                "UPDATE lt_documentos SET resumo=?,resumo_gerado=1,erro_resumo=NULL,resumo_cancelado=0 WHERE id=?",
                params![serde_json::to_string(&r)?, doc],
            )?,
            CampoDocumento::ErroResumo(e) => {
                c.execute("UPDATE lt_documentos SET erro_resumo=?,resumo_cancelado=0 WHERE id=?", params![e, doc])?
            }
            CampoDocumento::ResumoCancelado => c.execute("UPDATE lt_documentos SET resumo_cancelado=1 WHERE id=?", [doc])?,
        };
        Ok(())
    }

    /// Referência quebrada (arquivo ausente, incompleto ou com soma diferente) vira lacuna visível.
    pub fn marcar_lacuna(&self, id: &IdBloco, motivo: &str) -> SqlResult<()> {
        self.db.execute(
            "UPDATE lt_blocos SET estado='lacuna',erro=? WHERE id=? AND audio_liberado=0 AND estado<>'descartado'",
            params![motivo, id.texto()],
        )?;
        Ok(())
    }

    pub fn esquecer_arquivo(&self, id: &IdBloco) -> SqlResult<()> {
        self.db
            .execute("UPDATE lt_blocos SET arquivo=NULL WHERE id=?", [id.texto()])?;
        Ok(())
    }

    /// Arquivos que podem sair: áudio liberado depois do commit ou bloco descartado.
    pub fn arquivos_para_limpar(&self) -> SqlResult<Vec<(IdBloco, String)>> {
        let mut s = self.db.prepare(
            "SELECT documento,fonte,geracao,seq,arquivo FROM lt_blocos WHERE arquivo IS NOT NULL AND (audio_liberado=1 OR estado='descartado')",
        )?;
        let linhas = s.query_map([], |r| {
            Ok((
                IdBloco {
                    documento: r.get(0)?,
                    fonte: fonte(&r.get::<_, String>(1)?).unwrap_or(FonteDeAudio::Mac),
                    geracao: r.get(2)?,
                    seq: r.get(3)?,
                },
                r.get(4)?,
            ))
        })?;
        linhas.collect()
    }

    pub fn abrir_costura(
        &self,
        doc: &str,
        f: FonteDeAudio,
        geracao: u64,
        inicio: usize,
    ) -> SqlResult<()> {
        self.db.execute(
            "INSERT OR IGNORE INTO lt_costuras(documento,fonte,geracao,confirmado_ate) VALUES(?,?,?,?)",
            params![doc, f.nome(), geracao, inicio],
        )?;
        Ok(())
    }

    pub fn costuras(&self, doc: &str) -> SqlResult<Vec<EstadoCostura>> {
        let mut s = self
            .db
            .prepare("SELECT fonte,geracao,confirmado_ate,fechada FROM lt_costuras WHERE documento=? ORDER BY fonte,geracao")?;
        let linhas = s.query_map([doc], |r| {
            Ok(EstadoCostura {
                fonte: fonte(&r.get::<_, String>(0)?).unwrap_or(FonteDeAudio::Mac),
                geracao: r.get(1)?,
                confirmado_ate: r.get(2)?,
                fechada: r.get(3)?,
            })
        })?;
        linhas.collect()
    }

    /// Toma a costura para uma tentativa. Só um dono confirma de cada vez.
    pub fn tomar_costura(
        &self,
        doc: &str,
        f: FonteDeAudio,
        geracao: u64,
        dono: &str,
    ) -> SqlResult<()> {
        self.db.execute(
            "UPDATE lt_costuras SET dono=? WHERE documento=? AND fonte=? AND geracao=?",
            params![dono, doc, f.nome(), geracao],
        )?;
        Ok(())
    }

    pub fn confirmar(&mut self, p: &Passo) -> SqlResult<bool> {
        Ok(self.confirmar_persistindo(p)?.is_some())
    }

    /// Confirma trechos, avanço do ponto e estado dos blocos resolvidos numa transação. Exige o ponto
    /// e o dono esperados. Devolve os trechos com o ID gravado (sufixo em colisão; trecho idêntico já
    /// gravado não volta) ou `None` se outra tentativa avançou.
    pub fn confirmar_persistindo(&mut self, p: &Passo) -> SqlResult<Option<Vec<TrechoNovo>>> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if !existe(&tx, "SELECT 1 FROM lt_documentos WHERE id=? AND estado<>'descartada'", [&p.documento])?
            || !existe(
                &tx,
                "SELECT 1 FROM lt_costuras WHERE documento=? AND fonte=? AND geracao=? AND confirmado_ate=? AND dono=?",
                params![p.documento, p.fonte.nome(), p.geracao, p.esperado, p.dono],
            )?
        {
            return Ok(None);
        }
        let mut persistidos = vec![];
        for t in &p.trechos {
            if let Some(t) = inserir(&tx, &p.documento, "", t)? {
                persistidos.push(t);
            }
        }
        if let Some((inicio, fim, motivo)) = &p.pendencia {
            tx.execute(
                "INSERT OR IGNORE INTO lt_pendencias(id,documento,fonte,geracao,inicio,fim,motivo,estado) VALUES(?,?,?,?,?,?,?,'aberta')",
                params![
                    format!("{}|{}|{}|{}", p.documento, p.fonte.nome(), p.geracao, inicio),
                    p.documento,
                    p.fonte.nome(),
                    p.geracao,
                    inicio,
                    fim,
                    motivo
                ],
            )?;
        }
        tx.execute(
            "UPDATE lt_costuras SET confirmado_ate=?,fechada=? WHERE documento=? AND fonte=? AND geracao=?",
            params![p.novo, p.fechar, p.documento, p.fonte.nome(), p.geracao],
        )?;
        // Blocos capturados inteiros antes do ponto: pendentes se cruzam faixa não reconhecida,
        // transcritos se têm fala ou atividade, sem fala no resto.
        let abertos: Vec<(String, usize, usize)> = {
            let mut s = tx.prepare(
                "SELECT id,inicio,inicio+quantidade FROM lt_blocos WHERE documento=? AND fonte=? AND geracao=? AND estado='capturado' AND inicio+quantidade<=?",
            )?;
            let linhas = s.query_map(
                params![p.documento, p.fonte.nome(), p.geracao, p.novo],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?;
            linhas.collect::<SqlResult<_>>()?
        };
        for (id, inicio, fim) in abertos {
            let pendente = existe(
                &tx,
                "SELECT 1 FROM lt_pendencias WHERE documento=? AND fonte=? AND geracao=? AND estado='aberta' AND inicio<? AND fim>?",
                params![p.documento, p.fonte.nome(), p.geracao, fim, inicio],
            )?;
            let tem_fala = existe(
                &tx,
                "SELECT 1 FROM lt_trechos WHERE documento=? AND versao='' AND fonte=? AND (inicio+fim)*8000>=? AND (inicio+fim)*8000<?",
                params![p.documento, p.fonte.nome(), inicio, fim],
            )?;
            let estado = if pendente {
                EstadoBloco::Pendente
            } else if tem_fala || p.com_atividade.contains(&id) {
                EstadoBloco::Transcrito
            } else {
                EstadoBloco::SemFala
            };
            let erro = pendente
                .then_some("Fala não reconhecida. O áudio continua guardado para nova tentativa.");
            tx.execute(
                "UPDATE lt_blocos SET estado=?,erro=? WHERE id=? AND estado='capturado'",
                params![estado.nome(), erro, id],
            )?;
        }
        tx.commit()?;
        Ok(Some(persistidos))
    }

    pub fn pendencias(&self, doc: &str) -> SqlResult<Vec<Pendencia>> {
        pendencias_em(&self.db, doc)
    }

    /// Nova tentativa de uma faixa não reconhecida. Exige a pendência aberta com as tentativas esperadas.
    pub fn resolver(
        &mut self,
        id: &str,
        doc: &str,
        tentativas: u32,
        desfecho: DesfechoPendencia,
    ) -> SqlResult<bool> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if !existe(
            &tx,
            "SELECT 1 FROM lt_documentos WHERE id=? AND estado<>'descartada'",
            [doc],
        )? {
            return Ok(false);
        }
        let Some(p) = pendencias_em(&tx, doc)?.into_iter().find(|p| p.id == id) else {
            return Ok(false);
        };
        if p.estado != "aberta" || p.tentativas != tentativas {
            return Ok(false);
        }
        match desfecho {
            DesfechoPendencia::Resolvida(trechos) => {
                for t in &trechos {
                    inserir(&tx, doc, "", t)?;
                }
                tx.execute("UPDATE lt_pendencias SET estado='resolvida',tentativas=tentativas+1 WHERE id=?", [id])?;
            }
            DesfechoPendencia::SemFala => {
                tx.execute(
                    "UPDATE lt_pendencias SET estado='semFala',tentativas=tentativas+1 WHERE id=?",
                    [id],
                )?;
            }
            DesfechoPendencia::Continua(motivo) => {
                tx.execute(
                    "UPDATE lt_pendencias SET motivo=?,tentativas=tentativas+1 WHERE id=?",
                    params![motivo, id],
                )?;
                tx.commit()?;
                return Ok(true);
            }
        }
        liberar_pendentes(&tx, doc, p.fonte, p.geracao, p.inicio, p.fim)?;
        tx.commit()?;
        Ok(true)
    }

    pub fn versoes(&self, doc: &str) -> SqlResult<Vec<VersaoDetalhada>> {
        let mut s = self.db.prepare(
            "SELECT id,documento,tentativa,origem,estado,identidade FROM lt_versoes WHERE documento=? ORDER BY tentativa",
        )?;
        let linhas = s.query_map([doc], |r| {
            Ok(VersaoDetalhada {
                id: r.get(0)?,
                documento: r.get(1)?,
                tentativa: r.get(2)?,
                origem: r.get(3)?,
                estado: r.get(4)?,
                identidade: r.get(5)?,
            })
        })?;
        linhas.collect()
    }

    /// Lacunas e faixas pendentes da reunião, como aparecem no documento.
    pub fn periodos(&self, doc: &str) -> Result<(Vec<PeriodoDeAudio>, Vec<PeriodoDeAudio>), Erro> {
        let d = self.documento(doc)?;
        Ok(d.map(|d| {
            (
                d.lacunas.unwrap_or_default(),
                d.pendentes.unwrap_or_default(),
            )
        })
        .unwrap_or_default())
    }

    /// Roda na abertura, antes de listar recuperáveis: adota PCM completo sem registro, marca lacuna
    /// onde o registro aponta para arquivo ausente e repete a limpeza interrompida. Repetir não duplica.
    pub fn preparar_abertura(&mut self, disco: &DiscoDeBlocos) -> RelatorioAbertura {
        let mut r = RelatorioAbertura::default();
        if let Err(e) = self.reconciliar(disco, &mut r) {
            r.falhas.push(format!("reconciliação: {e}"));
        }
        r
    }

    fn reconciliar(
        &mut self,
        disco: &DiscoDeBlocos,
        r: &mut RelatorioAbertura,
    ) -> Result<(), Erro> {
        let registrados: HashSet<String> = {
            let mut s = self
                .db
                .prepare("SELECT arquivo FROM lt_blocos WHERE arquivo IS NOT NULL")?;
            let linhas = s.query_map([], |l| l.get(0))?;
            linhas.collect::<SqlResult<_>>()?
        };
        let (adotaveis, invalidos) = disco.orfaos(&registrados)?;
        r.invalidos = invalidos;
        // Costuras criadas por esta adoção: o ponto confirmado nunca passa à frente de áudio adotado.
        let mut nascidas = HashSet::new();
        for b in adotaveis {
            let doc = b.id.documento.clone();
            let chave = format!("{}|{}|{}", doc, b.id.fonte.nome(), b.id.geracao);
            if existe(
                &self.db,
                "SELECT 1 FROM lt_descartes WHERE documento=?",
                [&doc],
            )? {
                if let Some(a) = &b.arquivo {
                    disco.apagar(&doc, a)?;
                }
                continue;
            }
            let tx = self
                .db
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            if !existe(&tx, "SELECT 1 FROM lt_documentos WHERE id=?", [&doc])? {
                tx.execute(
                    "INSERT INTO lt_documentos(id,tipo,nome,criado_em,fonte,estado,falantes) VALUES(?,'reuniao','Reunião recuperada',?,'macEMicrofone','interrompida','pendente')",
                    params![doc, agora()],
                )?;
            }
            if tx.execute(
                "INSERT OR IGNORE INTO lt_costuras(documento,fonte,geracao,confirmado_ate) VALUES(?,?,?,?)",
                params![doc, b.id.fonte.nome(), b.id.geracao, b.inicio],
            )? > 0
            {
                nascidas.insert(chave.clone());
            } else if nascidas.contains(&chave) {
                tx.execute(
                    "UPDATE lt_costuras SET confirmado_ate=MIN(confirmado_ate,?) WHERE documento=? AND fonte=? AND geracao=?",
                    params![b.inicio, doc, b.id.fonte.nome(), b.id.geracao],
                )?;
            }
            tx.execute(
                "UPDATE lt_costuras SET fechada=0 WHERE documento=? AND fonte=? AND geracao=? AND confirmado_ate<?",
                params![doc, b.id.fonte.nome(), b.id.geracao, b.fim()],
            )?;
            let inserido = tx.execute(
                "INSERT OR IGNORE INTO lt_blocos(id,documento,fonte,geracao,seq,inicio,quantidade,arquivo,tamanho,soma,estado) VALUES(?,?,?,?,?,?,?,?,?,?,'capturado')",
                params![b.id.texto(), doc, b.id.fonte.nome(), b.id.geracao, b.id.seq, b.inicio, b.quantidade, b.arquivo, b.quantidade * 4, b.soma],
            )? == 1;
            if inserido {
                r.adotados += 1;
                proteger_adotado_atrasado(&tx, &b, nascidas.contains(&chave))?;
            }
            tx.commit()?;
        }
        let docs: Vec<String> = {
            let mut s = self.db.prepare(
                "SELECT DISTINCT documento FROM lt_blocos WHERE arquivo IS NOT NULL AND audio_liberado=0 AND estado<>'lacuna'",
            )?;
            let linhas = s.query_map([], |l| l.get(0))?;
            linhas.collect::<SqlResult<_>>()?
        };
        for doc in docs {
            for b in self.blocos(&doc)? {
                if !b.audio_liberado
                    && b.estado != EstadoBloco::Lacuna
                    && b.arquivo.is_some()
                    && !disco.existe(&b)
                {
                    self.marcar_lacuna(&b.id, MOTIVO_INCOMPLETO)?;
                    r.lacunas += 1;
                }
            }
        }
        for (id, arquivo) in self.arquivos_para_limpar()? {
            // Caminho inválido vindo do banco não é seguido: só a referência sai.
            if DiscoDeBlocos::arquivo_valido(&arquivo, &id.documento) {
                disco.apagar(&id.documento, &arquivo)?;
            }
            self.esquecer_arquivo(&id)?;
        }
        let descartadas: Vec<String> = {
            let mut s = self
                .db
                .prepare("SELECT id FROM lt_documentos WHERE estado='descartada'")?;
            let linhas = s.query_map([], |l| l.get(0))?;
            linhas.collect::<SqlResult<_>>()?
        };
        for doc in descartadas {
            disco.remover_pasta(&doc);
        }
        Ok(())
    }
}

/// Órfão adotado numa costura que já existia e cujo ponto confirmado passa à frente dele: a parte
/// anterior ao ponto vira pendência própria do intervalo inteiro e o PCM fica retido. Pendências
/// abertas sobrepostas são absorvidas. A costura não recua.
fn proteger_adotado_atrasado(
    c: &Connection,
    b: &BlocoRegistrado,
    costura_nova: bool,
) -> SqlResult<()> {
    if costura_nova {
        return Ok(());
    }
    let (doc, f, g) = (&b.id.documento, b.id.fonte.nome(), b.id.geracao);
    let Some(confirmado) = c
        .query_row(
            "SELECT confirmado_ate FROM lt_costuras WHERE documento=? AND fonte=? AND geracao=?",
            params![doc, f, g],
            |r| r.get::<_, usize>(0),
        )
        .optional()?
    else {
        return Ok(());
    };
    if b.inicio >= confirmado {
        return Ok(());
    }
    let (mut inicio, mut ate) = (b.inicio, b.fim().min(confirmado));
    let sobrepostas: Vec<(String, usize, usize)> = {
        let mut s = c.prepare(
            "SELECT id,inicio,fim FROM lt_pendencias WHERE documento=? AND fonte=? AND geracao=? AND estado='aberta' AND inicio<? AND fim>?",
        )?;
        let linhas = s.query_map(params![doc, f, g, ate, inicio], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?;
        linhas.collect::<SqlResult<_>>()?
    };
    for (_, i, z) in &sobrepostas {
        inicio = inicio.min(*i);
        ate = ate.max(*z);
    }
    for (id, _, _) in &sobrepostas {
        c.execute(
            "UPDATE lt_pendencias SET estado='absorvida' WHERE id=? AND estado='aberta'",
            [id],
        )?;
    }
    let motivo = "Áudio recuperado depois de uma interrupção, ainda não transcrito. O áudio continua guardado.";
    let id = format!("adocao|{doc}|{f}|{g}|{inicio}|{ate}");
    c.execute(
        "INSERT OR IGNORE INTO lt_pendencias(id,documento,fonte,geracao,inicio,fim,motivo,estado) VALUES(?,?,?,?,?,?,?,'aberta')",
        params![id, doc, f, g, inicio, ate, motivo],
    )?;
    c.execute("UPDATE lt_pendencias SET estado='aberta' WHERE id=?", [&id])?;
    if b.fim() <= confirmado {
        c.execute(
            "UPDATE lt_blocos SET estado='pendente',erro=? WHERE id=? AND estado='capturado'",
            params![motivo, b.id.texto()],
        )?;
    }
    Ok(())
}
