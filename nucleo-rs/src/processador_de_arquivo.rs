use crate::{motores::*, *};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, path::Path, time::Instant};
pub trait FonteDePCM {
    fn quantidade(&self) -> usize;
    fn ler(&self, inicio: usize, quantidade: usize) -> Result<Vec<f32>, Erro>;
}
impl FonteDePCM for Vec<f32> {
    fn quantidade(&self) -> usize {
        self.len()
    }
    fn ler(&self, inicio: usize, quantidade: usize) -> Result<Vec<f32>, Erro> {
        Ok(self
            .get(inicio..inicio.checked_add(quantidade).ok_or("Intervalo inválido")?)
            .ok_or("Janela incompleta")?
            .to_vec())
    }
}
pub struct AlinhamentoDeVoz;
impl AlinhamentoDeVoz {
    pub fn identificar(inicio: f64, fim: f64, faixas: &[FaixaDeVoz]) -> String {
        let mut ordenadas: Vec<_> = faixas
            .iter()
            .filter(|f| f.inicio < fim && f.fim > inicio)
            .collect();
        ordenadas.sort_by(|a, b| a.inicio.total_cmp(&b.inicio));
        let mut soma = HashMap::new();
        for f in ordenadas {
            let (total, ate) = soma.entry(&f.voz).or_insert((0., inicio));
            let a = f.inicio.max(inicio).max(*ate);
            let z = f.fim.min(fim);
            if z > a {
                *total += z - a;
                *ate = z
            }
        }
        let mut vozes: Vec<_> = soma.into_iter().collect();
        vozes.sort_by(|a, b| b.1 .0.total_cmp(&a.1 .0));
        if let Some((voz, (n, _))) = vozes.first() {
            if *n >= (fim - inicio) * 0.45 && *n - vozes.get(1).map_or(0., |v| v.1 .0) > 0.02 {
                return (*voz).clone();
            }
        }
        "desconhecida".into()
    }
}
pub(crate) fn recuperar_cauda(
    audio: &[f32],
    base: f64,
    ultimo: f64,
    r: &dyn ReconhecedorDeFala,
) -> Result<Option<(f64, Vec<PalavraReconhecida>)>, Erro> {
    let Some(c) = CaudaDaFala::localizar(audio) else {
        return Ok(None);
    };
    if base + c.fim as f64 / 16000. - ultimo < 0.28 {
        return Ok(None);
    }
    let limite = base + c.inicio as f64 / 16000. - 0.1;
    let ate = audio.len().min(c.fim + 3200);
    for margem in [1600, 16000, audio.len()] {
        let de = c.inicio.saturating_sub(margem);
        let palavras = r.palavras(
            &audio[de..ate],
            base + de as f64 / 16000.,
            Some((ate.min(c.fim + 1280) - de) as f64 / 16000.),
        )?;
        let uteis: Vec<_> = palavras
            .into_iter()
            .filter(|p| (p.inicio + p.fim) / 2. >= limite)
            .collect();
        if uteis.last().is_some_and(|p| p.fim > ultimo) {
            return Ok(Some((limite, uteis)));
        }
        if de == 0 {
            break;
        }
    }
    Ok(None)
}
pub struct ProcessadorDeArquivo<'a> {
    pub armazem: &'a mut ArmazemLancamento,
    pub reconhecedor: &'a dyn ReconhecedorDeFala,
    pub identificador: Option<&'a dyn IdentificadorDeFalantes>,
}
#[derive(Debug, PartialEq, Eq)]
pub enum Desfecho {
    Publicada,
    SemFala,
    Superada,
}
impl ProcessadorDeArquivo<'_> {
    pub fn transcrever(
        &mut self,
        v: &Versao,
        pcm: &dyn FonteDePCM,
        wav: Option<&Path>,
        preparar: impl Fn(&str) -> String,
        conferir: impl Fn() -> Result<(), Erro>,
        mut progresso: impl FnMut(&str, Option<f64>),
    ) -> Result<Desfecho, Erro> {
        let inicio = Instant::now();
        let total = pcm.quantidade();
        let mut hash = Sha256::new();
        let mut palavras: Vec<PalavraReconhecida> = vec![];
        progresso("transcrevendo", Some(0.));
        for (seq, i) in (0..total).step_by(240000).enumerate() {
            let fim = total.min(i + 240000);
            let de = i.saturating_sub(16000);
            let ate = total.min(fim + 16000);
            let audio = pcm.ler(de, ate - de)?;
            if audio.len() != ate - de {
                return Err("Janela incompleta".into());
            }
            let faixa = &audio[i - de..fim - de];
            let dados: Vec<_> = faixa.iter().flat_map(|s| s.to_le_bytes()).collect();
            hash.update(&dados);
            let janela = i as f64 / 16000. ..fim as f64 / 16000.;
            let mut novas = self
                .reconhecedor
                .palavras(&audio, de as f64 / 16000., None)?;
            novas.retain(|p| janela.contains(&((p.inicio + p.fim) / 2.)));
            if fim == total {
                let a = audio.len().saturating_sub(CaudaDaFala::CAPACIDADE);
                let base = (a + de) as f64 / 16000.;
                if let Some((limite, cauda)) = recuperar_cauda(
                    &audio[a..],
                    base,
                    novas.last().or(palavras.last()).map_or(base, |p| p.fim),
                    self.reconhecedor,
                )? {
                    novas.retain(|p| p.inicio < limite);
                    novas.extend(
                        cauda
                            .into_iter()
                            .filter(|p| janela.contains(&((p.inicio + p.fim) / 2.))),
                    )
                }
            }
            let pendente = novas.is_empty() && AtividadeDeVoz::tem(faixa);
            let estado = if pendente {
                EstadoBloco::Pendente
            } else if novas.is_empty() {
                EstadoBloco::SemFala
            } else {
                EstadoBloco::Transcrito
            };
            if !self.armazem.confirmar_janela(
                &v.id,
                seq,
                i,
                fim,
                &format!("{:x}", Sha256::digest(&dados)),
                estado,
            )? {
                return Ok(Desfecho::Superada);
            }
            if pendente {
                return Err("Há fala sem transcrição. Tente novamente.".into());
            }
            palavras.extend(novas);
            progresso("transcrevendo", Some(fim as f64 / total as f64))
        }
        let faixas = match (self.identificador, wav) {
            (Some(id), Some(w)) if !palavras.is_empty() => {
                progresso("diarizando", None);
                id.faixas(w)?
            }
            _ => vec![],
        };
        progresso("limpando", None);
        let mut trechos = Vec::<TrechoNovo>::new();
        for p in &palavras {
            let voz = AlinhamentoDeVoz::identificar(p.inicio, p.fim, &faixas);
            if let Some(t) = trechos.last_mut() {
                if t.voz == voz
                    && p.inicio - t.fim < 1.2
                    && p.fim - t.inicio <= 15.
                    && !t.texto.ends_with(['.', '!', '?'])
                {
                    t.texto.push(' ');
                    t.texto.push_str(&p.texto);
                    t.fim = t.fim.max(p.fim);
                    continue;
                }
            }
            trechos.push(TrechoNovo {
                id: format!("{}|{}|{}", v.documento, v.id, trechos.len()),
                fonte: FonteDeAudio::Arquivo,
                inicio: p.inicio,
                fim: p.fim,
                texto: p.texto.clone(),
                voz,
            })
        }
        for t in &mut trechos {
            let preparado = preparar(&t.texto);
            if !preparado.trim().is_empty() {
                t.texto = preparado
            }
        }
        conferir()?;
        let vazia = trechos.is_empty();
        let baixa_confianca =
            self.identificador.is_some() && trechos.iter().any(|t| t.voz == "desconhecida");
        let ok = self.armazem.publicar(&Publicacao {
            versao: v.id.clone(),
            identidade: format!("{:x}", hash.finalize()),
            trechos,
            duracao: total as f64 / 16000.,
            bruto: palavras
                .into_iter()
                .map(|p| p.texto)
                .collect::<Vec<_>>()
                .join(" "),
            processamento: inicio.elapsed().as_secs_f64(),
            baixa_confianca,
            quantidade_pcm: Some(total),
        })?;
        Ok(if ok {
            progresso("limpando", Some(1.));
            Desfecho::Publicada
        } else if vazia {
            Desfecho::SemFala
        } else {
            Desfecho::Superada
        })
    }
}
