use crate::{motores::Erro, *};
use rusqlite::{params, OptionalExtension};
use std::collections::{HashMap, HashSet};
impl ArmazemLancamento {
    pub fn documento(&self, id: &str) -> Result<Option<TranscricaoLonga>, Erro> {
        let tx = self.db.unchecked_transaction()?;
        let d = self.montar(id, false)?;
        tx.commit()?;
        Ok(d)
    }
    fn montar(&self, id: &str, leve: bool) -> Result<Option<TranscricaoLonga>, Erro> {
        let Some((tipo,versao,mut d))=self.db.query_row("SELECT tipo,versao_ativa,nome,criado_em,duracao,estado,fonte,falantes,oculto,erro_captura,aviso_captura,aviso,atraso,erro_resumo,resumo_cancelado,resumo,resumo_gerado FROM lt_documentos WHERE id=?",[id],|r|{let duracao=r.get::<_,f64>(4)?;let resumo=r.get::<_,Option<String>>(15)?;Ok((r.get::<_,String>(0)?,r.get::<_,Option<String>>(1)?,TranscricaoLonga{id:id.into(),nome:r.get(2)?,criado_em:r.get(3)?,duracao:(duracao>0.).then_some(duracao),fase:r.get(5)?,fonte:r.get(6)?,falantes:r.get(7)?,oculto:r.get::<_,bool>(8)?.then_some(true),erro_captura:r.get(9)?,aviso_captura:r.get(10)?,aviso:r.get(11)?,atraso:r.get(12)?,erro_resumo:r.get(13)?,resumo_cancelado:r.get::<_,bool>(14)?.then_some(true),resumo:resumo.and_then(|j|serde_json::from_str(&j).ok()),resumo_gerado:r.get::<_,bool>(16)?.then_some(true),..Default::default()}))}).optional()? else{return Ok(None)};
        if tipo == "arquivo" {
            if let Some(v) = &versao {
                d.fase = "pronto".into();
                (d.duracao,d.bruto,d.processamento,d.baixa_confianca)=self.db.query_row("SELECT duracao,CASE WHEN ? THEN NULL ELSE bruto END,round(processamento,1),baixa_confianca FROM lt_versoes WHERE id=?",params![leve,v],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?
            } else {
                let linha=self.db.query_row("SELECT estado,erro FROM lt_versoes WHERE documento=? ORDER BY tentativa DESC LIMIT 1",[id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,Option<String>>(1)?))).optional()?;
                let (estado, erro) = linha.unwrap_or_default();
                d.fase = match estado.as_str() {
                    "vazia" => "semFala",
                    "cancelada" | "superada" | "" => "cancelado",
                    _ => "falhou",
                }
                .into();
                d.motivo = if estado == "emCurso" {
                    Some("A transcrição foi interrompida. Tente de novo.".into())
                } else {
                    erro
                }
            }
        } else {
            if d.fonte.is_none() {
                d.fonte = Some("mac".into())
            }
            d.fase = match &*d.fase {
                "finalizada" => "pronta".into(),
                "processando" => "finalizando".into(),
                _ => d.fase,
            };
            let mut pendentes = vec![];
            let mut lacunas = vec![];
            let mut segundos = 0.;
            let mut blocos = self.blocos(id)?;
            blocos.sort_by_key(|b| (b.id.fonte.nome(), b.inicio));
            for b in blocos {
                let fim = (b.inicio + b.quantidade) as f64 / 16000.;
                d.duracao = Some(d.duracao.unwrap_or(0.).max(fim));
                if matches!(b.estado, EstadoBloco::Capturado | EstadoBloco::Pendente) {
                    segundos += b.quantidade as f64 / 16000.
                }
                let (lista, motivo) = match b.estado {
                    EstadoBloco::Lacuna => (
                        &mut lacunas,
                        b.erro
                            .unwrap_or_else(|| "Sem áudio neste intervalo.".into()),
                    ),
                    EstadoBloco::Capturado => (
                        &mut pendentes,
                        "Áudio guardado, ainda não transcrito.".into(),
                    ),
                    _ => continue,
                };
                let inicio = b.inicio as f64 / 16000.;
                let fonte = b.id.fonte.nome();
                if let Some(p) = lista.last_mut().filter(|p: &&mut PeriodoDeAudio| {
                    p.fonte == fonte && p.motivo == motivo && (p.fim - inicio).abs() < 0.01
                }) {
                    p.fim = fim
                } else {
                    lista.push(PeriodoDeAudio {
                        inicio,
                        fim,
                        motivo,
                        fonte: fonte.into(),
                    })
                }
            }
            let mut s=self.db.prepare("SELECT inicio/16000.0,fim/16000.0,fonte,motivo FROM lt_pendencias WHERE documento=? AND estado='aberta'")?;
            for p in s.query_map([id], |r| {
                Ok(PeriodoDeAudio {
                    inicio: r.get(0)?,
                    fim: r.get(1)?,
                    fonte: r.get(2)?,
                    motivo: r.get(3)?,
                })
            })? {
                pendentes.push(p?)
            }
            pendentes.sort_by(|a, b| a.inicio.total_cmp(&b.inicio));
            d.segundos_pendentes = (segundos > 0.).then_some(segundos);
            d.pendentes = (!pendentes.is_empty()).then_some(pendentes);
            d.lacunas = (!lacunas.is_empty()).then_some(lacunas)
        }
        let versao = versao.unwrap_or_default();
        let mut s = self.db.prepare(
            "SELECT tipo,alvo,valor FROM lt_edicoes WHERE documento=? AND versao=? ORDER BY id",
        )?;
        let mut nomes = HashMap::new();
        let mut unioes = HashMap::new();
        let mut manuais = HashMap::new();
        let mut rows = s.query(params![id, versao])?;
        while let Some(r) = rows.next()? {
            let k = r.get::<_, String>(1)?;
            let v = r.get::<_, String>(2)?;
            match &*r.get::<_, String>(0)? {
                "uniao" => {
                    unioes.insert(k, v);
                }
                "nomeVoz" => {
                    unioes.remove(&k);
                    nomes.insert(k, v);
                }
                "vozTrecho" => {
                    manuais.insert(k, v);
                }
                _ => {}
            }
        }
        let mut s=self.db.prepare("SELECT id,fonte,inicio,fim,CASE WHEN ? THEN '' ELSE texto END,COALESCE(voz_final,voz),divergente FROM lt_trechos WHERE documento=? AND versao=? ORDER BY inicio,fonte,id")?;
        let mut rows = s.query(params![leve, id, versao])?;
        let mut vistos = HashSet::new();
        let mut pessoas = 0;
        while let Some(r) = rows.next()? {
            let tid: String = r.get(0)?;
            let manual = manuais.get(&tid);
            let mut voz = manual.cloned().unwrap_or(r.get(5)?);
            let mut caminho = HashSet::new();
            while let Some(v) = unioes.get(&voz) {
                if !caminho.insert(v.clone()) {
                    break;
                }
                voz = v.clone()
            }
            if vistos.insert(voz.clone()) {
                let nome = match voz.as_str() {
                    "voce" => "Você".into(),
                    "chamada" => "Chamada".into(),
                    "desconhecida" => "Voz não identificada".into(),
                    _ => {
                        pessoas += 1;
                        format!("Pessoa {pessoas}")
                    }
                };
                d.vozes.push(VozLonga {
                    id: voz.clone(),
                    nome: nomes.get(&voz).cloned().unwrap_or(nome),
                    nome_manual: nomes.contains_key(&voz).then_some(true),
                })
            }
            if tipo == "reuniao" && voz == "desconhecida" {
                d.baixa_confianca = true
            }
            if !leve {
                d.trechos.push(TrechoLongo {
                    id: tid,
                    inicio: r.get(2)?,
                    fim: r.get(3)?,
                    voz,
                    texto: r.get(4)?,
                    origem: (r.get::<_, String>(1)? == "mic").then(|| "microfone".into()),
                    voz_manual: manual.is_some().then_some(true),
                    divergente: r.get::<_, bool>(6)?.then_some(true),
                    ..Default::default()
                })
            }
        }
        if leve {
            d.palavras=Some(self.db.query_row("SELECT COALESCE(SUM(CASE WHEN trim(texto)='' THEN 0 ELSE length(trim(texto))-length(replace(trim(texto),' ',''))+1 END),0) FROM lt_trechos WHERE documento=? AND versao=?",params![id,versao],|r|r.get(0))?);
            d.previa=self.db.query_row("SELECT substr(texto,1,240) FROM lt_trechos WHERE documento=? AND versao=? ORDER BY inicio LIMIT 1",params![id,versao],|r|r.get(0)).optional()?
        }
        Ok(Some(d))
    }
    fn listar(&self, sql: &str, arg: Option<&str>) -> Result<Vec<TranscricaoLonga>, Erro> {
        let tx = self.db.unchecked_transaction()?;
        let mut lista = vec![];
        {
            let mut s = tx.prepare(sql)?;
            for id in s.query_map([arg], |r| r.get::<_, String>(0))? {
                if let Some(d) = self.montar(&id?, true)? {
                    lista.push(d)
                }
            }
        }
        tx.commit()?;
        Ok(lista)
    }
    pub fn lista(&self, tipo: &str) -> Result<Vec<TranscricaoLonga>, Erro> {
        self.listar("SELECT id FROM lt_documentos WHERE tipo=? AND estado<>'descartada' ORDER BY criado_em DESC",Some(tipo))
    }
    pub fn recuperaveis(&self, ativa: Option<&str>) -> Result<Vec<TranscricaoLonga>, Erro> {
        self.listar("SELECT id FROM lt_documentos d WHERE id IS NOT ? AND tipo='reuniao' AND estado<>'descartada' AND (estado<>'finalizada' OR EXISTS(SELECT 1 FROM lt_blocos b WHERE b.documento=d.id AND (b.estado IN ('capturado','pendente') OR (d.falantes IN ('pendente','indisponivel') AND b.audio_liberado=0 AND b.fonte='mac' AND b.estado<>'lacuna')))) ORDER BY criado_em DESC",ativa)
    }
    pub fn renomear(&mut self, doc: &str, voz: &str, nome: &str) -> Result<(), Erro> {
        let tx = self.db.unchecked_transaction()?;
        let d = self.montar(doc, true)?.ok_or("Documento inexistente")?;
        let nome: String = nome.trim().chars().take(120).collect();
        if nome.is_empty() {
            return Err("Nome vazio".into());
        }
        let destino = d.vozes.iter().find(|v| {
            v.id != voz && Palavras::normalizadas(&v.nome) == Palavras::normalizadas(&nome)
        });
        tx.execute("INSERT INTO lt_edicoes(documento,versao,tipo,alvo,valor,criado_em) SELECT id,coalesce(versao_ativa,''),?,?,?,? FROM lt_documentos WHERE id=?",params![if destino.is_some(){"uniao"}else{"nomeVoz"},voz,destino.map_or(&nome,|v|&v.id),armazem_lancamento::agora(),doc])?;
        tx.commit()?;
        Ok(())
    }
    pub fn atribuir_nova_pessoa(
        &mut self,
        doc: &str,
        trecho: &str,
        nome: &str,
    ) -> Result<(), Erro> {
        let tx = self.db.unchecked_transaction()?;
        let nome: String = nome.trim().chars().take(120).collect();
        if nome.is_empty() {
            return Err("Nome vazio".into());
        }
        let d = self.montar(doc, true)?.ok_or("Documento inexistente")?;
        let voz = if let Some(v) = d
            .vozes
            .iter()
            .find(|v| Palavras::normalizadas(&v.nome) == Palavras::normalizadas(&nome))
        {
            v.id.clone()
        } else {
            let voz = tx.query_row("SELECT 'manual-'||lower(hex(randomblob(16)))", [], |r| {
                r.get::<_, String>(0)
            })?;
            tx.execute("INSERT INTO lt_edicoes(documento,versao,tipo,alvo,valor,criado_em) SELECT documento,versao,'nomeVoz',?,?,? FROM lt_trechos WHERE documento=? AND id=?",params![voz,nome,armazem_lancamento::agora(),doc,trecho])?;
            voz
        };
        self.reatribuir(doc, trecho, &voz)?;
        tx.commit()?;
        Ok(())
    }
}
