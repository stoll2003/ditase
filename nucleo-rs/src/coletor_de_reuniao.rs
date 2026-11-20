use crate::{
    motores::{Erro, TAXA},
    *,
};
use std::collections::HashMap;
struct Fonte {
    geracao: u64,
    seq: u64,
    cursor: CursorDeFonte,
    buffer: Vec<f32>,
    inicio: usize,
}
pub struct ColetorDeReuniao {
    pub documento: String,
    pub armazem: ArmazemLancamento,
    pub disco: DiscoDeBlocos,
    pub linha_do_tempo: LinhaDoTempo,
    fontes: HashMap<FonteDeAudio, Fonte>,
    tamanho_bloco: usize,
    encerrado: bool,
}
impl ColetorDeReuniao {
    pub fn new(
        documento: String,
        armazem: ArmazemLancamento,
        disco: DiscoDeBlocos,
        inicio: f64,
    ) -> Self {
        Self {
            documento,
            armazem,
            disco,
            linha_do_tempo: LinhaDoTempo::new(inicio),
            fontes: HashMap::new(),
            tamanho_bloco: TAXA as usize,
            encerrado: false,
        }
    }
    pub fn abrir(&mut self, fonte: FonteDeAudio, t: f64) -> Result<u64, Erro> {
        if self.encerrado {
            return Err("Coletor encerrado".into());
        }
        let mut abertura = self
            .linha_do_tempo
            .indice(t)
            .unwrap_or_else(|| self.fontes.get(&fonte).map_or(0, |f| f.cursor.proximo));
        let mut geracao = 0;
        if self.fontes.contains_key(&fonte) {
            self.selar(fonte, None)?;
            let f = &self.fontes[&fonte];
            geracao = f.geracao + 1;
            abertura = abertura.max(f.cursor.proximo);
            if abertura - f.cursor.proximo > 960 {
                self.lacuna(fonte, f.cursor.proximo as usize, abertura as usize)?
            }
        }
        self.armazem.db.execute(
            "INSERT INTO lt_costuras(documento,fonte,geracao,confirmado_ate) VALUES(?,?,?,?)",
            rusqlite::params![self.documento, fonte.nome(), geracao, abertura],
        )?;
        self.fontes.insert(
            fonte,
            Fonte {
                geracao,
                seq: 0,
                cursor: CursorDeFonte::new(abertura),
                buffer: vec![],
                inicio: abertura as usize,
            },
        );
        Ok(geracao)
    }
    fn lacuna(&mut self, fonte: FonteDeAudio, inicio: usize, fim: usize) -> Result<(), Erro> {
        let f = self.fontes.get_mut(&fonte).ok_or("Fonte fechada")?;
        let b = BlocoRegistrado {
            id: IdBloco {
                documento: self.documento.clone(),
                fonte,
                geracao: f.geracao,
                seq: f.seq,
            },
            inicio,
            quantidade: fim - inicio,
            arquivo: None,
            soma: None,
            estado: EstadoBloco::Lacuna,
            tentativas: 0,
            erro: Some("A fonte não entregou áudio neste intervalo.".into()),
            audio_liberado: false,
        };
        if !self.armazem.registrar(&b)? {
            return Err("Conflito de bloco".into());
        }
        f.seq += 1;
        f.inicio = fim;
        f.cursor.proximo = fim as i64;
        Ok(())
    }
    fn selar(&mut self, fonte: FonteDeAudio, quantidade: Option<usize>) -> Result<(), Erro> {
        let f = self.fontes.get_mut(&fonte).ok_or("Fonte fechada")?;
        let n = quantidade.unwrap_or(f.buffer.len());
        if n == 0 {
            return Ok(());
        }
        let id = IdBloco {
            documento: self.documento.clone(),
            fonte,
            geracao: f.geracao,
            seq: f.seq,
        };
        let b = self.disco.gravar(id, f.inicio, &f.buffer[..n])?;
        if !self.armazem.registrar(&b)? {
            return Err("Conflito no registro de áudio".into());
        }
        f.buffer.drain(..n);
        f.inicio += n;
        f.seq += 1;
        Ok(())
    }
    pub fn receber(
        &mut self,
        fonte: FonteDeAudio,
        tempo: f64,
        amostras: &[f32],
    ) -> Result<(), Erro> {
        if self.encerrado {
            return Ok(());
        }
        let Some(f) = self.fontes.get_mut(&fonte) else {
            return Ok(());
        };
        // Calcule primeiro; aplique o cursor só após aceitar cada evento.
        let mut cursor = CursorDeFonte::new(f.cursor.proximo);
        let eventos = cursor.receber(tempo, amostras.len(), &self.linha_do_tempo);
        for e in eventos {
            match e {
                Evento::Lacuna(r) => {
                    self.selar(fonte, None)?;
                    self.lacuna(fonte, r.start as usize, r.end as usize)?;
                }
                Evento::Amostras(p) => {
                    let f = self.fontes.get_mut(&fonte).ok_or("Fonte fechada")?;
                    if f.buffer.is_empty() {
                        f.inicio = p.indice as usize
                    }
                    f.buffer
                        .extend_from_slice(&amostras[p.origem..p.origem + p.quantidade]);
                    // Se selar falhar, as amostras já estão no buffer para nova tentativa.
                    f.cursor.proximo = p.indice + p.quantidade as i64;
                    while self.fontes[&fonte].buffer.len() >= self.tamanho_bloco {
                        self.selar(fonte, Some(self.tamanho_bloco))?
                    }
                }
            }
        }
        Ok(())
    }
    pub fn selar_tudo(&mut self) -> Result<(), Erro> {
        let mut falha = None;
        for fonte in self.fontes.keys().copied().collect::<Vec<_>>() {
            if let Err(e) = self.selar(fonte, None) {
                falha = Some(e)
            }
        }
        falha.map_or(Ok(()), Err)
    }
    pub fn pausar(&mut self, t: f64) -> Result<(), Erro> {
        self.linha_do_tempo.pausar(t);
        self.selar_tudo()
    }
    pub fn retomar(&mut self, t: f64) {
        self.linha_do_tempo.retomar(t)
    }
    pub fn marcar_fim(&mut self, t: f64) {
        self.linha_do_tempo.pausar(t)
    }
    pub fn encerrar(&mut self, t: f64) -> Result<(), Erro> {
        self.marcar_fim(t);
        self.selar_tudo()?;
        self.encerrado = true;
        Ok(())
    }
    pub fn amostras_em_memoria(&self) -> usize {
        self.fontes.values().map(|f| f.buffer.len()).sum()
    }
    pub fn fim_atual(&self) -> i64 {
        self.fontes
            .values()
            .map(|f| f.cursor.proximo)
            .max()
            .unwrap_or(0)
    }
}
