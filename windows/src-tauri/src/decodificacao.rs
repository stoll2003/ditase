//! Decodificação de arquivos para PCM 16 kHz mono (porta da leitura de `AudioLancamento.swift`).
//! Vídeo e áudio de celular (mp4, mov, m4a) vão pelo Media Foundation do Windows, que traz os
//! codecs do sistema; wav, flac e mp3 vão pelo Symphonia (Rust puro). Se o Media Foundation
//! falhar, o Symphonia tenta (AAC em MP4 ele também lê).
use std::{fs::File, path::Path};

use symphonia::core::{
    audio::SampleBuffer, codecs::DecoderOptions, errors::Error as ErroSym, formats::FormatOptions,
    io::MediaSourceStream, meta::MetadataOptions, probe::Hint,
};

use crate::audio::Reamostrador;

pub type Erro = Box<dyn std::error::Error + Send + Sync>;

pub const SUPORTADOS: &[&str] = &["wav", "flac", "mp3", "m4a", "mp4", "mov", "aac"];

pub fn extensao(caminho: &Path) -> String {
    caminho
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

pub fn suportado(caminho: &Path) -> bool {
    SUPORTADOS.contains(&extensao(caminho).as_str())
}

/// Lê o arquivo inteiro e devolve PCM 16 kHz mono.
pub fn decodificar(caminho: &Path) -> Result<Vec<f32>, Erro> {
    let ext = extensao(caminho);
    if matches!(ext.as_str(), "mp4" | "mov" | "m4a") {
        match crate::nativo::midia::decodificar(caminho) {
            Ok(pcm) => return Ok(pcm),
            Err(e) => {
                crate::caminhos::nota(format!("media foundation falhou ({e}); tentando symphonia"))
            }
        }
    }
    symphonia(caminho, &ext)
}

/// Mistura canais intercalados em mono.
pub fn para_mono(intercalado: &[f32], canais: usize) -> Vec<f32> {
    if canais <= 1 {
        return intercalado.to_vec();
    }
    intercalado
        .chunks(canais)
        .map(|q| q.iter().sum::<f32>() / canais as f32)
        .collect()
}

fn symphonia(caminho: &Path, ext: &str) -> Result<Vec<f32>, Erro> {
    let fonte = MediaSourceStream::new(Box::new(File::open(caminho)?), Default::default());
    let mut dica = Hint::new();
    dica.with_extension(ext);
    let sondado = symphonia::default::get_probe().format(
        &dica,
        fonte,
        &FormatOptions::default(),
        &MetadataOptions::default(),
    )?;
    let mut formato = sondado.format;
    let faixa = formato
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)
        .ok_or("o arquivo não tem faixa de áudio")?;
    let id = faixa.id;
    let mut decodificador =
        symphonia::default::get_codecs().make(&faixa.codec_params, &DecoderOptions::default())?;
    let mut reamostrar: Option<Reamostrador> = None;
    let mut saida = Vec::new();
    loop {
        let pacote = match formato.next_packet() {
            Ok(p) => p,
            Err(ErroSym::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(ErroSym::ResetRequired) => break,
            Err(e) => return Err(e.into()),
        };
        if pacote.track_id() != id {
            continue;
        }
        match decodificador.decode(&pacote) {
            Ok(bloco) => {
                let spec = *bloco.spec();
                let mut amostras = SampleBuffer::<f32>::new(bloco.capacity() as u64, spec);
                amostras.copy_interleaved_ref(bloco);
                let mono = para_mono(amostras.samples(), spec.channels.count());
                let r = reamostrar.get_or_insert_with(|| Reamostrador::new(spec.rate));
                saida.extend(r.processar(&mono));
            }
            // Pacote corrompido: pula, como o AVFoundation faz.
            Err(ErroSym::DecodeError(_)) => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(saida)
}
