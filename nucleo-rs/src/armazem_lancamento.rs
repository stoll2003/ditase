use crate::{
    banco::abrir, disco_de_blocos::fonte, motores::Erro, BlocoRegistrado, DiscoDeBlocos,
    EstadoBloco, TrechoNovo,
};
use rusqlite::{params, Connection, OptionalExtension, Result as SqlResult, TransactionBehavior};
use std::{collections::HashMap, path::Path};
pub struct ArmazemLancamento {
    pub(crate) db: Connection,
}
pub(crate) fn agora() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}
pub(crate) fn inserir(
    c: &Connection,
    doc: &str,
    versao: &str,
    t: &TrechoNovo,
) -> SqlResult<Option<TrechoNovo>> {
    let mut id = t.id.clone();
    let mut n = 0;
    loop {
        let igual=c.prepare_cached("SELECT documento=? AND versao=? AND fonte=? AND inicio=? AND fim=? AND texto=? FROM lt_trechos WHERE id=?")?.query_row(params![doc,versao,t.fonte.nome(),t.inicio,t.fim,t.texto,id],|r|r.get::<_,bool>(0)).optional()?;
        match igual {
            Some(true) => return Ok(None),
            Some(false) => {
                n += 1;
                id = format!("{}#{n}", t.id)
            }
            None => break,
        }
    }
    c.prepare_cached("INSERT INTO lt_trechos(id,documento,versao,fonte,inicio,fim,texto,voz,criado_em) VALUES(?,?,?,?,?,?,?,?,?)")?.execute(params![id,doc,versao,t.fonte.nome(),t.inicio,t.fim,t.texto,t.voz,agora()])?;
    Ok(Some(TrechoNovo { id, ..t.clone() }))
}
impl ArmazemLancamento {
    pub fn new(caminho: impl AsRef<Path>) -> Result<Self, Erro> {
        let db = abrir(caminho.as_ref())?;
        db.execute_batch(include_str!("lancamento.sql"))?;
        Ok(Self { db })
    }
    pub fn criar_documento(
        &self,
        id: &str,
        tipo: &str,
        nome: &str,
        fonte: Option<&str>,
        estado: &str,
    ) -> SqlResult<()> {
        self.db.execute("INSERT INTO lt_documentos(id,tipo,nome,criado_em,fonte,estado,falantes) VALUES(?,?,?,?,?,?,?) ON CONFLICT(id) DO NOTHING",params![id,tipo,nome,agora(),fonte,estado,if tipo=="arquivo"{"naoAplica"}else{"pendente"}])?;
        Ok(())
    }
    pub fn estado_documento(&self, id: &str) -> SqlResult<Option<String>> {
        self.db
            .query_row("SELECT estado FROM lt_documentos WHERE id=?", [id], |r| {
                r.get(0)
            })
            .optional()
    }
    pub fn registrar(&self, b: &BlocoRegistrado) -> SqlResult<bool> {
        let id = &b.id;
        Ok(self.db.prepare_cached("INSERT INTO lt_blocos(id,documento,fonte,geracao,seq,inicio,quantidade,arquivo,tamanho,soma,estado,tentativas,erro,audio_liberado) SELECT ?,?,?,?,?,?,?,?,?,?,?,?,?,? WHERE NOT EXISTS (SELECT 1 FROM lt_descartes WHERE documento=?) ON CONFLICT(id) DO NOTHING")?.execute(params![id.texto(),id.documento,id.fonte.nome(),id.geracao,id.seq,b.inicio,b.quantidade,b.arquivo,b.arquivo.as_ref().map(|_|b.quantidade*4),b.soma,b.estado.nome(),b.tentativas,b.erro,b.audio_liberado,id.documento])?==1)
    }
    pub fn blocos(&self, doc: &str) -> SqlResult<Vec<BlocoRegistrado>> {
        Self::blocos_em(&self.db, doc)
    }

    pub(crate) fn blocos_em(db: &Connection, doc: &str) -> SqlResult<Vec<BlocoRegistrado>> {
        Self::consultar_blocos(
            db,
            doc,
            "SELECT fonte,geracao,seq,inicio,quantidade,arquivo,soma,estado,tentativas,erro,audio_liberado FROM lt_blocos WHERE documento=? ORDER BY fonte,geracao,seq",
            [doc],
        )
    }

    pub(crate) fn blocos_na_janela(
        &self,
        doc: &str,
        fonte: crate::FonteDeAudio,
        geracao: u64,
        de: usize,
        ate: usize,
    ) -> SqlResult<Vec<BlocoRegistrado>> {
        Self::consultar_blocos(
            &self.db,
            doc,
            "SELECT fonte,geracao,seq,inicio,quantidade,arquivo,soma,estado,tentativas,erro,audio_liberado FROM lt_blocos WHERE documento=? AND fonte=? AND geracao=? AND inicio+quantidade>? AND inicio<? ORDER BY seq",
            params![doc, fonte.nome(), geracao, de, ate],
        )
    }

    fn consultar_blocos(
        db: &Connection,
        doc: &str,
        sql: &str,
        parametros: impl rusqlite::Params,
    ) -> SqlResult<Vec<BlocoRegistrado>> {
        let mut s = db.prepare_cached(sql)?;
        let rows = s.query_map(parametros, |r| {
            Ok(BlocoRegistrado {
                id: crate::IdBloco {
                    documento: doc.into(),
                    fonte: fonte(&r.get::<_, String>(0)?).ok_or(rusqlite::Error::InvalidQuery)?,
                    geracao: r.get(1)?,
                    seq: r.get(2)?,
                },
                inicio: r.get(3)?,
                quantidade: r.get(4)?,
                arquivo: r.get(5)?,
                soma: r.get(6)?,
                estado: match &*r.get::<_, String>(7)? {
                    "capturado" => EstadoBloco::Capturado,
                    "transcrito" => EstadoBloco::Transcrito,
                    "semFala" => EstadoBloco::SemFala,
                    "pendente" => EstadoBloco::Pendente,
                    "lacuna" => EstadoBloco::Lacuna,
                    "descartado" => EstadoBloco::Descartado,
                    _ => return Err(rusqlite::Error::InvalidQuery),
                },
                tentativas: r.get(8)?,
                erro: r.get(9)?,
                audio_liberado: r.get(10)?,
            })
        })?;
        rows.collect()
    }
    pub fn trechos(&self, doc: &str) -> SqlResult<Vec<TrechoNovo>> {
        let mut s=self.db.prepare("SELECT t.id,t.fonte,t.inicio,t.fim,t.texto,COALESCE((SELECT valor FROM lt_edicoes WHERE documento=t.documento AND versao=t.versao AND tipo='vozTrecho' AND alvo=t.id ORDER BY id DESC LIMIT 1),t.voz_final,t.voz) FROM lt_trechos t JOIN lt_documentos d ON d.id=t.documento WHERE d.id=? AND t.versao=COALESCE(d.versao_ativa,'') ORDER BY t.inicio,t.fim,t.id")?;
        let rows = s.query_map([doc], |r| {
            Ok(TrechoNovo {
                id: r.get(0)?,
                fonte: fonte(&r.get::<_, String>(1)?).ok_or(rusqlite::Error::InvalidQuery)?,
                inicio: r.get(2)?,
                fim: r.get(3)?,
                texto: r.get(4)?,
                voz: r.get(5)?,
            })
        })?;
        rows.collect()
    }
    pub fn reatribuir(&self, doc: &str, trecho: &str, voz: &str) -> SqlResult<()> {
        Self::reatribuir_em(&self.db, doc, trecho, voz)
    }

    pub(crate) fn reatribuir_em(
        db: &Connection,
        doc: &str,
        trecho: &str,
        voz: &str,
    ) -> SqlResult<()> {
        if db.execute("INSERT INTO lt_edicoes(documento,versao,tipo,alvo,valor,criado_em) SELECT documento,versao,'vozTrecho',id,?,? FROM lt_trechos WHERE documento=? AND id=?",params![voz,agora(),doc,trecho])?==0{return Err(rusqlite::Error::QueryReturnedNoRows)}
        Ok(())
    }
    pub fn gravar_falantes(
        &mut self,
        doc: &str,
        atribuicao: &HashMap<String, String>,
        identificados: bool,
        liberar_audio: bool,
    ) -> SqlResult<bool> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mudou = tx.execute(
            "UPDATE lt_documentos SET falantes=?1,estado='finalizada' WHERE id=?2 AND estado<>'descartada'
             AND NOT EXISTS(SELECT 1 FROM lt_blocos WHERE documento=?2 AND estado IN ('capturado','pendente'))
             AND NOT EXISTS(SELECT 1 FROM lt_pendencias WHERE documento=?2 AND estado='aberta')",
            params![if identificados { "identificados" } else { "indisponivel" }, doc],
        )?;
        if mudou == 0 {
            return Ok(false);
        }
        {
            let mut atualizar =
                tx.prepare("UPDATE lt_trechos SET voz_final=? WHERE documento=? AND id=?")?;
            for (id, voz) in atribuicao {
                atualizar.execute(params![voz, doc, id])?;
            }
        }
        if liberar_audio {
            tx.execute(
                "UPDATE lt_blocos SET audio_liberado=1
                 WHERE documento=? AND audio_liberado=0 AND estado IN ('transcrito','semFala','lacuna')
                 AND NOT EXISTS (SELECT 1 FROM lt_pendencias p WHERE p.documento=lt_blocos.documento
                    AND p.fonte=lt_blocos.fonte AND p.geracao=lt_blocos.geracao AND p.estado='retida'
                    AND p.inicio<lt_blocos.inicio+lt_blocos.quantidade AND p.fim>lt_blocos.inicio)",
                [doc],
            )?;
        }
        tx.commit()?;
        Ok(true)
    }
    pub fn descartar(&mut self, doc: &str) -> SqlResult<bool> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if tx.execute(
            "INSERT OR IGNORE INTO lt_descartes(documento) SELECT id FROM lt_documentos WHERE id=?",
            [doc],
        )? == 0
        {
            return Ok(false);
        }
        tx.execute("UPDATE lt_blocos SET estado=CASE WHEN estado IN ('capturado','pendente') THEN 'descartado' ELSE estado END,audio_liberado=1 WHERE documento=?",[doc])?;
        tx.execute(
            "UPDATE lt_pendencias SET estado='descartada' WHERE documento=? AND estado IN ('aberta','retida')",
            [doc],
        )?;
        tx.execute(
            "UPDATE lt_costuras SET dono=NULL,fechada=1 WHERE documento=?",
            [doc],
        )?;
        tx.execute("UPDATE lt_documentos SET estado=CASE WHEN EXISTS(SELECT 1 FROM lt_trechos WHERE documento=?1) THEN 'finalizada' ELSE 'descartada' END,falantes=CASE WHEN falantes='identificados' THEN falantes ELSE 'indisponivel' END WHERE id=?1",[doc])?;
        tx.commit()?;
        Ok(true)
    }
    pub fn limpar(&self, doc: &str, disco: &DiscoDeBlocos) -> Result<(), Erro> {
        for b in self.blocos(doc)? {
            if let Some(a) = b
                .arquivo
                .filter(|_| b.audio_liberado || b.estado == EstadoBloco::Descartado)
            {
                disco.apagar(doc, &a)?;
                self.db.execute(
                    "UPDATE lt_blocos SET arquivo=NULL WHERE id=? AND documento=? AND arquivo=?",
                    params![b.id.texto(), doc, a],
                )?;
            }
        }
        Ok(())
    }
    pub fn criar_versao(&mut self, doc: &str, nome: &str, origem: &str) -> SqlResult<Versao> {
        self.criar_documento(doc, "arquivo", nome, None, "ativo")?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let v=tx.query_row("SELECT lower(hex(randomblob(16))),COALESCE(MAX(tentativa),0)+1 FROM lt_versoes WHERE documento=?",[doc],|r|Ok(Versao{id:r.get(0)?,documento:doc.into(),tentativa:r.get(1)?}))?;
        tx.execute("INSERT INTO lt_versoes (id,documento,tentativa,origem,estado,criada_em) VALUES (?,?,?,?,'emCurso',?)",params![v.id,doc,v.tentativa,origem,agora()])?;
        tx.commit()?;
        Ok(v)
    }
    pub fn encerrar_versao(&self, id: &str, estado: &str, erro: Option<&str>) -> SqlResult<bool> {
        if !matches!(estado, "cancelada" | "falhou" | "superada") {
            return Err(rusqlite::Error::InvalidQuery);
        }
        Ok(self.db.execute(
            "UPDATE lt_versoes SET estado=?,erro=? WHERE id=? AND estado='emCurso'",
            params![estado, erro, id],
        )? == 1)
    }
    pub fn confirmar_janela(
        &self,
        versao: &str,
        seq: usize,
        inicio: usize,
        fim: usize,
        soma: &str,
        estado: EstadoBloco,
    ) -> SqlResult<bool> {
        Ok(self.db.execute("INSERT INTO lt_janelas_arquivo(versao,seq,inicio,fim,soma,estado) SELECT id,?,?,?,?,? FROM lt_versoes WHERE id=? AND estado='emCurso'",params![seq,inicio,fim,soma,estado.nome(),versao])?==1)
    }
    pub fn versao_ativa(&self, doc: &str) -> SqlResult<Option<String>> {
        Ok(self
            .db
            .query_row(
                "SELECT versao_ativa FROM lt_documentos WHERE id=?",
                [doc],
                |r| r.get(0),
            )
            .optional()?
            .flatten())
    }
    pub fn publicar(&mut self, p: &Publicacao) -> Result<bool, Erro> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some((doc, n)) = tx
            .query_row(
                "SELECT documento,tentativa FROM lt_versoes WHERE id=? AND estado='emCurso'",
                [&p.versao],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
            )
            .optional()?
        else {
            return Ok(false);
        };
        if tx.query_row("SELECT EXISTS(SELECT 1 FROM lt_versoes WHERE documento=? AND tentativa>? AND estado IN ('pronta','emCurso'))",params![doc,n],|r|r.get::<_,bool>(0))?{tx.execute("UPDATE lt_versoes SET estado='superada' WHERE id=?",[&p.versao])?;tx.commit()?;return Ok(false)}
        if let Some(total) = p.quantidade_pcm {
            let mut fim = 0;
            let mut s = tx.prepare(
                "SELECT inicio,fim,estado FROM lt_janelas_arquivo WHERE versao=? ORDER BY seq",
            )?;
            let mut rows = s.query([&p.versao])?;
            while let Some(r) = rows.next()? {
                let ate = r.get::<_, usize>(1)?;
                if r.get::<_, usize>(0)? != fim
                    || ate <= fim
                    || !matches!(&*r.get::<_, String>(2)?, "semFala" | "transcrito")
                {
                    return Err("Janela sem transcrição".into());
                }
                fim = ate
            }
            if fim != total {
                return Err("Arquivo incompleto".into());
            }
        }
        if p.trechos.is_empty() {
            tx.execute(
                "UPDATE lt_versoes SET estado='vazia',identidade=? WHERE id=?",
                params![p.identidade, p.versao],
            )?;
            tx.commit()?;
            return Ok(false);
        }
        for t in &p.trechos {
            inserir(&tx, &doc, &p.versao, t)?;
        }
        tx.execute("UPDATE lt_versoes SET estado='pronta',identidade=?,publicada_em=?,duracao=?,bruto=?,processamento=?,baixa_confianca=? WHERE id=?",params![p.identidade,agora(),p.duracao,p.bruto,p.processamento,p.baixa_confianca,p.versao])?;
        tx.execute(
            "UPDATE lt_documentos SET versao_ativa=?,duracao=?,oculto=0 WHERE id=?",
            params![p.versao, p.duracao, doc],
        )?;
        tx.commit()?;
        Ok(true)
    }
}
#[derive(Debug, Clone)]
pub struct Versao {
    pub id: String,
    pub documento: String,
    pub tentativa: i64,
}
pub struct Publicacao {
    pub versao: String,
    pub identidade: String,
    pub trechos: Vec<TrechoNovo>,
    pub duracao: f64,
    pub bruto: String,
    pub processamento: f64,
    pub baixa_confianca: bool,
    pub quantidade_pcm: Option<usize>,
}
