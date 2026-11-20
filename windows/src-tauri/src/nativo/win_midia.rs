//! Media Foundation: decodifica mp4, mov e m4a com os codecs do Windows. Pede PCM float na taxa
//! e nos canais originais; a mistura para mono e a redução para 16 kHz são nossas (as mesmas do
//! microfone), para não depender do reamostrador de cada versão do Windows.
use std::path::Path;

use windows::{
    core::HSTRING,
    Win32::{
        Media::MediaFoundation::{
            IMFMediaBuffer, IMFSample, MFAudioFormat_Float, MFCreateMediaType,
            MFCreateSourceReaderFromURL, MFMediaType_Audio, MFShutdown, MFStartup, MFSTARTUP_FULL,
            MF_MT_AUDIO_NUM_CHANNELS, MF_MT_AUDIO_SAMPLES_PER_SECOND, MF_MT_MAJOR_TYPE,
            MF_MT_SUBTYPE, MF_SOURCE_READERF_ENDOFSTREAM, MF_SOURCE_READER_ALL_STREAMS,
            MF_SOURCE_READER_FIRST_AUDIO_STREAM, MF_VERSION,
        },
        System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED},
    },
};

use crate::{audio::Reamostrador, decodificacao::para_mono};

struct Sessao;
impl Drop for Sessao {
    fn drop(&mut self) {
        unsafe {
            let _ = MFShutdown();
            CoUninitialize();
        }
    }
}

pub fn decodificar(caminho: &Path) -> Result<Vec<f32>, String> {
    let erro = |e: windows::core::Error| e.message().to_string();
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED)
            .ok()
            .map_err(erro)?;
        MFStartup(MF_VERSION, MFSTARTUP_FULL).map_err(erro)?;
        let _sessao = Sessao;
        let leitor =
            MFCreateSourceReaderFromURL(&HSTRING::from(caminho.as_os_str()), None).map_err(erro)?;
        let audio = MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32;
        leitor
            .SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false)
            .map_err(erro)?;
        leitor
            .SetStreamSelection(audio, true)
            .map_err(|_| "o arquivo não tem faixa de áudio".to_string())?;
        let pedido = MFCreateMediaType().map_err(erro)?;
        pedido
            .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)
            .map_err(erro)?;
        pedido
            .SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_Float)
            .map_err(erro)?;
        leitor
            .SetCurrentMediaType(audio, None, &pedido)
            .map_err(erro)?;
        let atual = leitor.GetCurrentMediaType(audio).map_err(erro)?;
        let taxa = atual
            .GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND)
            .map_err(erro)?;
        let canais = atual
            .GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS)
            .map_err(erro)?
            .max(1) as usize;
        let mut reamostrar = Reamostrador::new(taxa);
        let mut saida = Vec::new();
        loop {
            let mut bandeiras = 0u32;
            let mut amostra: Option<IMFSample> = None;
            leitor
                .ReadSample(
                    audio,
                    0,
                    None,
                    Some(&mut bandeiras),
                    None,
                    Some(&mut amostra),
                )
                .map_err(erro)?;
            if bandeiras & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                break;
            }
            let Some(amostra) = amostra else { continue };
            let buffer: IMFMediaBuffer = amostra.ConvertToContiguousBuffer().map_err(erro)?;
            let mut p = std::ptr::null_mut::<u8>();
            let mut tamanho = 0u32;
            buffer
                .Lock(&mut p, None, Some(&mut tamanho))
                .map_err(erro)?;
            let floats = std::slice::from_raw_parts(p as *const f32, tamanho as usize / 4);
            saida.extend(reamostrar.processar(&para_mono(floats, canais)));
            let _ = buffer.Unlock();
        }
        Ok(saida)
    }
}
