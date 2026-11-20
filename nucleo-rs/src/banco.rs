use crate::{motores::Erro, Termo};
use rusqlite::{params, Connection, OptionalExtension, Result as SqlResult, Row};
use std::path::Path;
pub(crate) fn abrir(caminho: &Path) -> Result<Connection, Erro> {
    if let Some(p) = caminho.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(p)?
    }
    #[cfg(unix)]
    if caminho != Path::new(":memory:") {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(caminho)?;
    }
    let c = Connection::open(caminho)?;
    c.execute_batch("PRAGMA busy_timeout=3000; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON")?;
    Ok(c)
}
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ditado {
    pub id: i64,
    pub iniciado_em: f64,
    pub duracao: f64,
    pub duracao_fala: f64,
    pub palavras: i64,
    pub texto_bruto: String,
    pub texto: String,
    pub app: String,
    pub app_nome: String,
    pub estado: String,
    pub latencia: f64,
    pub motor: String,
    pub limpador: String,
    pub trocas_dicionario: i64,
    pub origem: String,
    pub audio: String,
}
impl Default for Ditado {
    fn default() -> Self {
        Self {
            id: Default::default(),
            iniciado_em: Default::default(),
            duracao: Default::default(),
            duracao_fala: Default::default(),
            palavras: Default::default(),
            texto_bruto: Default::default(),
            texto: Default::default(),
            app: Default::default(),
            app_nome: Default::default(),
            estado: "gravando".into(),
            latencia: Default::default(),
            motor: Default::default(),
            limpador: Default::default(),
            trocas_dicionario: Default::default(),
            origem: "ditado".into(),
            audio: Default::default(),
        }
    }
}
pub struct Banco {
    db: Connection,
}
fn ler(r: &Row<'_>) -> SqlResult<Ditado> {
    Ok(Ditado {
        id: r.get(0)?,
        iniciado_em: r.get(1)?,
        duracao: r.get(2)?,
        duracao_fala: r.get(3)?,
        palavras: r.get(4)?,
        texto_bruto: r.get(5)?,
        texto: r.get(6)?,
        app: r.get(7)?,
        app_nome: r.get(8)?,
        estado: r.get(9)?,
        latencia: r.get(10)?,
        motor: r.get(11)?,
        limpador: r.get(12)?,
        trocas_dicionario: r.get(13)?,
        origem: r.get(14)?,
        audio: r.get(15)?,
    })
}
const COLUNAS:&str="id,iniciado_em,duracao,duracao_fala,palavras,texto_bruto,texto,app,app_nome,estado,latencia,motor,limpador,trocas_dicionario,origem,audio";
fn atualizar(c: &Connection, d: &Ditado) -> SqlResult<usize> {
    c.execute("UPDATE ditados SET duracao=?,duracao_fala=?,palavras=?,texto_bruto=?,texto=?,app=?,app_nome=?,estado=?,latencia=?,motor=?,limpador=?,trocas_dicionario=?,audio=? WHERE id=?",params![d.duracao,d.duracao_fala,d.palavras,d.texto_bruto,d.texto,d.app,d.app_nome,d.estado,d.latencia,d.motor,d.limpador,d.trocas_dicionario,d.audio,d.id])
}
impl Banco {
    pub fn new(caminho: impl AsRef<Path>) -> Result<Self, Erro> {
        let db = abrir(caminho.as_ref())?;
        db.execute_batch(include_str!("banco.sql"))?;
        Ok(Self { db })
    }
    pub fn inserir(&self, d: &Ditado, chave_externa: Option<&str>) -> SqlResult<i64> {
        self.db.execute("INSERT INTO ditados (iniciado_em,duracao,duracao_fala,palavras,texto_bruto,texto,app,app_nome,estado,latencia,motor,limpador,trocas_dicionario,origem,audio,chave_externa) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",params![d.iniciado_em,d.duracao,d.duracao_fala,d.palavras,d.texto_bruto,d.texto,d.app,d.app_nome,d.estado,d.latencia,d.motor,d.limpador,d.trocas_dicionario,d.origem,d.audio,chave_externa])?;
        Ok(self.db.last_insert_rowid())
    }
    pub fn existe(&self, chave_externa: &str) -> SqlResult<bool> {
        self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM ditados WHERE chave_externa=?)",
            [chave_externa],
            |r| r.get(0),
        )
    }
    pub fn atualizar(&self, d: &Ditado) -> SqlResult<()> {
        atualizar(&self.db, d)?;
        Ok(())
    }
    pub fn concluir(&mut self, d: &Ditado) -> SqlResult<()> {
        let tx = self.db.transaction()?;
        if atualizar(&tx, d)? != 1 {
            return Err(rusqlite::Error::QueryReturnedNoRows);
        }
        tx.execute("DELETE FROM trechos WHERE ditado_id=?", [d.id])?;
        tx.commit()
    }
    pub fn ditado(&self, id: i64) -> SqlResult<Option<Ditado>> {
        self.db
            .query_row(
                &format!("SELECT {COLUNAS} FROM ditados WHERE id=?"),
                [id],
                ler,
            )
            .optional()
    }
    fn lista(&self, condicao: &str, p: impl rusqlite::Params) -> SqlResult<Vec<Ditado>> {
        let mut s = self
            .db
            .prepare(&format!("SELECT {COLUNAS} FROM ditados {condicao}"))?;
        let rows = s.query_map(p, ler)?;
        rows.collect()
    }
    pub fn historico(
        &self,
        limite: i64,
        antes_de: Option<f64>,
        busca: &str,
    ) -> SqlResult<Vec<Ditado>> {
        let padrao = format!(
            "%{}%",
            busca
                .replace('!', "!!")
                .replace('%', "!%")
                .replace('_', "!_")
        );
        self.lista("WHERE estado IN ('concluido','parcial','recuperado','falhou') AND (?1 IS NULL OR iniciado_em<?1) AND texto LIKE ?2 ESCAPE '!' ORDER BY iniciado_em DESC LIMIT ?3",params![antes_de,padrao,limite])
    }
    pub fn ultimo_concluido(&self) -> SqlResult<Option<Ditado>> {
        Ok(self.lista("WHERE estado IN ('concluido','parcial','recuperado') AND origem='ditado' ORDER BY iniciado_em DESC LIMIT 1",[])?.pop())
    }
    pub fn para_estatisticas(&self) -> SqlResult<Vec<Ditado>> {
        self.lista("WHERE estado IN ('concluido','parcial','recuperado') AND palavras>0 ORDER BY iniciado_em",[])
    }
    pub fn interrompidos(&self) -> SqlResult<Vec<Ditado>> {
        self.lista("WHERE estado='gravando'", [])
    }
    pub fn gravar_trecho(&self, ditado: i64, ordem: i64, bruto: &str) -> SqlResult<()> {
        self.db.execute(
            "INSERT INTO trechos(ditado_id,ordem,bruto) VALUES(?,?,?)",
            params![ditado, ordem, bruto],
        )?;
        Ok(())
    }
    pub fn trechos(&self, ditado: i64) -> SqlResult<Vec<String>> {
        let mut s = self
            .db
            .prepare("SELECT bruto FROM trechos WHERE ditado_id=? ORDER BY ordem")?;
        let rows = s.query_map([ditado], |r| r.get(0))?;
        rows.collect()
    }
    pub fn apagar_ditado(&self, id: i64) -> SqlResult<()> {
        self.db.execute("DELETE FROM ditados WHERE id=?", [id])?;
        Ok(())
    }
    pub fn apagar_trechos(&self, id: i64) -> SqlResult<()> {
        self.db
            .execute("DELETE FROM trechos WHERE ditado_id=?", [id])?;
        Ok(())
    }
    pub fn apagar_termo(&self, id: i64) -> SqlResult<()> {
        self.db.execute("DELETE FROM dicionario WHERE id=?", [id])?;
        Ok(())
    }
    pub fn termos(&self) -> SqlResult<Vec<Termo>> {
        let mut s=self.db.prepare("SELECT id,termo,ouvido_como,usos,origem,criado_em FROM dicionario ORDER BY termo COLLATE NOCASE")?;
        let rows = s.query_map([], ler_termo)?;
        rows.collect()
    }
    pub fn salvar(&mut self, mut termo: Termo) -> Result<Termo, Erro> {
        termo.termo = termo.termo.trim().into();
        if termo.termo.is_empty() {
            return Err("Termo vazio".into());
        }
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if termo.id == 0 {
            if let Some(mut antigo)=tx.query_row("SELECT id,termo,ouvido_como,usos,origem,criado_em FROM dicionario WHERE termo=?",[&termo.termo],ler_termo).optional()?{antigo.ouvido_como.extend(termo.ouvido_como);antigo.termo=termo.termo;termo=antigo}
        }
        let mut vistos = std::collections::HashSet::from([termo.termo.to_lowercase()]);
        termo.ouvido_como = termo
            .ouvido_como
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty() && vistos.insert(s.to_lowercase()))
            .map(str::to_owned)
            .collect();
        let json = serde_json::to_string(&termo.ouvido_como)?;
        if termo.id != 0 {
            tx.execute(
                "UPDATE dicionario SET termo=?,ouvido_como=? WHERE id=?",
                params![termo.termo, json, termo.id],
            )?;
        } else {
            tx.execute(
                "INSERT INTO dicionario(termo,ouvido_como,usos,origem,criado_em) VALUES(?,?,?,?,?)",
                params![termo.termo, json, termo.usos, termo.origem, termo.criado_em],
            )?;
            termo.id = tx.last_insert_rowid()
        }
        tx.commit()?;
        Ok(termo)
    }
    pub fn contar_uso(&mut self, ids: &[i64]) -> SqlResult<()> {
        let tx = self.db.transaction()?;
        {
            let mut atualizar =
                tx.prepare_cached("UPDATE dicionario SET usos=usos+1 WHERE id=?")?;
            for id in ids {
                atualizar.execute([id])?;
            }
        }
        tx.commit()
    }
    pub fn config(&self, chave: &str) -> SqlResult<Option<String>> {
        self.db
            .query_row("SELECT valor FROM config WHERE chave=?", [chave], |r| {
                r.get(0)
            })
            .optional()
    }
    pub fn definir(&self, chave: &str, valor: &str) -> SqlResult<()> {
        self.db.execute("INSERT INTO config(chave,valor) VALUES(?,?) ON CONFLICT(chave) DO UPDATE SET valor=excluded.valor",[chave,valor])?;
        Ok(())
    }
}
fn ler_termo(r: &Row<'_>) -> SqlResult<Termo> {
    Ok(Termo {
        id: r.get(0)?,
        termo: r.get(1)?,
        ouvido_como: serde_json::from_str(&r.get::<_, String>(2)?).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(2, rusqlite::types::Type::Text, Box::new(e))
        })?,
        usos: r.get(3)?,
        origem: r.get(4)?,
        criado_em: r.get(5)?,
    })
}

impl Banco {
    pub fn salvar_transcricao(&self, t: &crate::TranscricaoLonga) -> Result<(), Erro> {
        let dados = serde_json::to_string(t)?;
        self.db.execute("INSERT INTO transcricoes_longas(id,tipo,criado_em,dados) VALUES(?,?,?,?) ON CONFLICT(id) DO UPDATE SET dados=excluded.dados",params![t.id,if t.fonte.is_some(){"reuniao"}else{"arquivo"},t.criado_em,dados])?;
        Ok(())
    }
    pub fn transcricao(&self, id: &str) -> Result<Option<crate::TranscricaoLonga>, Erro> {
        let s = self
            .db
            .query_row(
                "SELECT dados FROM transcricoes_longas WHERE id=?",
                [id],
                |r| r.get::<_, String>(0),
            )
            .optional()?;
        Ok(s.map(|j| serde_json::from_str(&j)).transpose()?)
    }
    pub fn transcricoes(
        &self,
        tipo: &str,
        detalhes: bool,
    ) -> Result<Vec<crate::TranscricaoLonga>, Erro> {
        let selecao = if detalhes {
            "dados"
        } else {
            "json_set(json_remove(dados,'$.bruto'),'$.trechos',json('[]'),'$.palavras',(SELECT coalesce(sum(CASE WHEN trim(json_extract(value,'$.texto'))='' THEN 0 ELSE length(trim(json_extract(value,'$.texto')))-length(replace(trim(json_extract(value,'$.texto')),' ',''))+1 END),0) FROM json_each(dados,'$.trechos')),'$.previa',substr(json_extract(dados,'$.trechos[0].texto'),1,240))"
        };
        let mut s = self.db.prepare(&format!(
            "SELECT {selecao} FROM transcricoes_longas WHERE tipo=? ORDER BY criado_em DESC"
        ))?;
        let rows = s.query_map([tipo], |r| r.get::<_, String>(0))?;
        rows.map(|j| Ok(serde_json::from_str(&j?)?)).collect()
    }
}
