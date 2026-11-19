//! Leitura de WAV para o formato do app (16 kHz, mono, f32) e caminhos para a API C.
use crate::{erro, Erro, TAXA};
use std::path::Path;

/// Lê um WAV PCM (16, 24 ou 32 bits inteiros, ou float 32), mistura os canais e reamostra para
/// 16 kHz se preciso. Em Rust, para não depender do `fopen` da API C com caminho não ASCII.
pub fn ler_wav(caminho: &Path) -> Result<Vec<f32>, Erro> {
    let dados =
        std::fs::read(caminho).map_err(|e| erro(format!("não li {}: {e}", caminho.display())))?;
    let (taxa, mono) = decodificar_wav(&dados)?;
    if taxa == TAXA {
        return Ok(mono);
    }
    let r = sherpa_onnx::LinearResampler::create(taxa as i32, TAXA as i32)
        .ok_or_else(|| erro("reamostrador indisponível"))?;
    Ok(r.resample(&mono, true))
}

fn decodificar_wav(d: &[u8]) -> Result<(u32, Vec<f32>), Erro> {
    let u16le = |i: usize| u16::from_le_bytes([d[i], d[i + 1]]);
    let u32le = |i: usize| u32::from_le_bytes([d[i], d[i + 1], d[i + 2], d[i + 3]]);
    if d.len() < 12 || &d[0..4] != b"RIFF" || &d[8..12] != b"WAVE" {
        return Err(erro("arquivo não é WAV"));
    }
    let mut i = 12;
    let mut fmt: Option<(u16, u16, u32, u16)> = None;
    while i + 8 <= d.len() {
        let id = &d[i..i + 4];
        let tam = u32le(i + 4) as usize;
        let corpo = i + 8;
        let fim = (corpo + tam).min(d.len());
        if id == b"fmt " && tam >= 16 {
            let mut formato = u16le(corpo);
            if formato == 0xFFFE && tam >= 26 {
                formato = u16le(corpo + 24); // WAVE_FORMAT_EXTENSIBLE: subformato
            }
            fmt = Some((
                formato,
                u16le(corpo + 2),
                u32le(corpo + 4),
                u16le(corpo + 14),
            ));
        } else if id == b"data" {
            let (formato, canais, taxa, bits) =
                fmt.ok_or_else(|| erro("WAV sem bloco fmt antes de data"))?;
            let canais = canais.max(1) as usize;
            let bytes = (bits / 8) as usize;
            let bruto = &d[corpo..fim];
            let amostra = |b: &[u8]| -> Option<f32> {
                match (formato, bits) {
                    (1, 16) => Some(i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0),
                    (1, 24) => Some(
                        ((i32::from_le_bytes([0, b[0], b[1], b[2]])) >> 8) as f32 / 8_388_608.0,
                    ),
                    (1, 32) => {
                        Some(i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32 / 2_147_483_648.0)
                    }
                    (3, 32) => Some(f32::from_le_bytes([b[0], b[1], b[2], b[3]])),
                    _ => None,
                }
            };
            if !matches!((formato, bits), (1, 16) | (1, 24) | (1, 32) | (3, 32)) {
                return Err(erro(format!(
                    "formato de WAV não suportado ({formato}, {bits} bits)"
                )));
            }
            let quadro = bytes * canais;
            let mut mono = Vec::with_capacity(bruto.len() / quadro.max(1));
            for q in bruto.chunks_exact(quadro) {
                let soma: f32 = q.chunks_exact(bytes).filter_map(amostra).sum();
                mono.push(soma / canais as f32);
            }
            return Ok((taxa, mono));
        }
        i = corpo + tam + (tam & 1);
    }
    Err(erro("WAV sem bloco data"))
}

/// Grava PCM 16 kHz mono num WAV de 16 bits.
pub fn gravar_wav(caminho: &Path, amostras: &[f32]) -> Result<(), Erro> {
    let n = amostras.len() as u32 * 2;
    let mut v = Vec::with_capacity(44 + n as usize);
    v.extend_from_slice(b"RIFF");
    v.extend_from_slice(&(36 + n).to_le_bytes());
    v.extend_from_slice(b"WAVEfmt ");
    v.extend_from_slice(&16u32.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes());
    v.extend_from_slice(&TAXA.to_le_bytes());
    v.extend_from_slice(&(TAXA * 2).to_le_bytes());
    v.extend_from_slice(&2u16.to_le_bytes());
    v.extend_from_slice(&16u16.to_le_bytes());
    v.extend_from_slice(b"data");
    v.extend_from_slice(&n.to_le_bytes());
    for a in amostras {
        v.extend_from_slice(&((a.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    std::fs::write(caminho, v).map_err(|e| erro(format!("não gravei {}: {e}", caminho.display())))
}

/// Caminho para entregar à API C do sherpa-onnx, que abre arquivos com `char*`.
/// No Windows isso usa a página de código ANSI: pasta com acento (`C:\Users\João`) quebraria.
/// Se o caminho não for ASCII, usa o nome curto 8.3 do Windows; o app também deve declarar
/// `activeCodePage UTF-8` no manifesto (ver LICENCAS.md, seção Windows).
pub fn caminho_c(caminho: &Path) -> Result<String, Erro> {
    let texto = caminho
        .to_str()
        .ok_or_else(|| erro(format!("caminho inválido: {}", caminho.display())))?;
    if texto.is_ascii() || cfg!(not(windows)) {
        return Ok(texto.to_string());
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::{OsStrExt, OsStringExt};
        let largo: Vec<u16> = caminho.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut buf = vec![0u16; 1024];
        let n = unsafe {
            windows_sys::Win32::Storage::FileSystem::GetShortPathNameW(
                largo.as_ptr(),
                buf.as_mut_ptr(),
                buf.len() as u32,
            )
        } as usize;
        if n > 0 && n < buf.len() {
            let curto = std::ffi::OsString::from_wide(&buf[..n]);
            if let Some(s) = curto.to_str().filter(|s| s.is_ascii()) {
                return Ok(s.to_string());
            }
        }
    }
    Ok(texto.to_string())
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn grava_e_le_wav() {
        let pasta = std::env::temp_dir().join(format!("motores-wav-{}", std::process::id()));
        std::fs::create_dir_all(&pasta).unwrap();
        let arq = pasta.join("t.wav");
        let sinal: Vec<f32> = (0..1600).map(|i| (i as f32 / 10.0).sin() * 0.5).collect();
        gravar_wav(&arq, &sinal).unwrap();
        let lido = ler_wav(&arq).unwrap();
        assert_eq!(lido.len(), sinal.len());
        assert!(lido.iter().zip(&sinal).all(|(a, b)| (a - b).abs() < 1e-3));
        let _ = std::fs::remove_dir_all(&pasta);
    }

    #[test]
    fn mistura_estereo_e_reamostra() {
        // 8 kHz, estéreo, 16 bits: canal esquerdo 0.5, direito 0 -> mono 0.25, dobro de amostras.
        let mut v = Vec::new();
        let n = 800u32;
        v.extend_from_slice(b"RIFF");
        v.extend_from_slice(&(36 + n * 4).to_le_bytes());
        v.extend_from_slice(b"WAVEfmt ");
        v.extend_from_slice(&16u32.to_le_bytes());
        for x in [1u16, 2] {
            v.extend_from_slice(&x.to_le_bytes());
        }
        v.extend_from_slice(&8000u32.to_le_bytes());
        v.extend_from_slice(&32000u32.to_le_bytes());
        v.extend_from_slice(&4u16.to_le_bytes());
        v.extend_from_slice(&16u16.to_le_bytes());
        v.extend_from_slice(b"data");
        v.extend_from_slice(&(n * 4).to_le_bytes());
        for _ in 0..n {
            v.extend_from_slice(&16384i16.to_le_bytes());
            v.extend_from_slice(&0i16.to_le_bytes());
        }
        let (taxa, mono) = decodificar_wav(&v).unwrap();
        assert_eq!(taxa, 8000);
        assert!((mono[10] - 0.25).abs() < 1e-3);
        let pasta = std::env::temp_dir().join(format!("motores-wav8-{}", std::process::id()));
        std::fs::create_dir_all(&pasta).unwrap();
        std::fs::write(pasta.join("e.wav"), &v).unwrap();
        let lido = ler_wav(&pasta.join("e.wav")).unwrap();
        assert!((lido.len() as i64 - 1600).abs() < 20, "{}", lido.len());
        let _ = std::fs::remove_dir_all(&pasta);
    }
}
