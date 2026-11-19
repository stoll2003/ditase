//! Catálogo dos modelos de fala e falantes (e do llama-server para Windows), no mesmo padrão
//! do `ModeloRemoto` do Swift: URL fixada numa revisão, tamanho e SHA-256 do arquivo LFS.
//!
//! O núcleo baixa com o seu baixador (retomada, espaço em disco, marcador de conferência).
//! [`baixar`] aqui é a versão mínima, usada pelos testes, pela medição e pelo CI.
use crate::{erro, Erro};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArquivoRemoto {
    /// Nome do arquivo dentro da pasta do conjunto.
    pub arquivo: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
    pub tamanho: u64,
}

/// Conjunto de arquivos que formam um modelo, numa subpasta própria da pasta de modelos.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConjuntoDeModelo {
    pub pasta: &'static str,
    pub licenca: &'static str,
    pub arquivos: &'static [ArquivoRemoto],
}

impl ConjuntoDeModelo {
    pub fn tamanho(&self) -> u64 {
        self.arquivos.iter().map(|a| a.tamanho).sum()
    }
    pub fn caminho(&self, modelos: &Path) -> PathBuf {
        modelos.join(self.pasta)
    }
    /// Todos os arquivos presentes com o tamanho certo (o hash é conferido no download).
    pub fn presente(&self, modelos: &Path) -> bool {
        self.arquivos.iter().all(|a| {
            std::fs::metadata(self.caminho(modelos).join(a.arquivo))
                .map(|m| m.len() == a.tamanho)
                .unwrap_or(false)
        })
    }
}

/// Parakeet TDT 0.6B v3 (NVIDIA, CC-BY-4.0), exportado para ONNX INT8 pelo projeto sherpa-onnx.
pub const PARAKEET: ConjuntoDeModelo = ConjuntoDeModelo {
    pasta: "parakeet",
    licenca: "CC-BY-4.0 (nvidia/parakeet-tdt-0.6b-v3)",
    arquivos: &[
        ArquivoRemoto {
            arquivo: "encoder.int8.onnx",
            url: "https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8/resolve/ed5d745623472f97cfc2e6c2916ee248798fc5a8/encoder.int8.onnx",
            sha256: "1afbe69c1ffdabee94755f42e4daf1d93549131a3da11356bda84fcef0f2c479",
            tamanho: 652_282_294,
        },
        ArquivoRemoto {
            arquivo: "decoder.int8.onnx",
            url: "https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8/resolve/ed5d745623472f97cfc2e6c2916ee248798fc5a8/decoder.int8.onnx",
            sha256: "0222a86147299d4dee90edf95b6d13827edb42ad8b7c9cf0b2582f030937c0b5",
            tamanho: 11_845_274,
        },
        ArquivoRemoto {
            arquivo: "joiner.int8.onnx",
            url: "https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8/resolve/ed5d745623472f97cfc2e6c2916ee248798fc5a8/joiner.int8.onnx",
            sha256: "cbd02f3696e9944970111fb439d9c3fd365a80374565f13f2b9c7007d253fc98",
            tamanho: 6_355_277,
        },
        ArquivoRemoto {
            arquivo: "tokens.txt",
            url: "https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8/resolve/ed5d745623472f97cfc2e6c2916ee248798fc5a8/tokens.txt",
            sha256: "d58544679ea4bc6ac563d1f545eb7d474bd6cfa467f0a6e2c1dc1c7d37e3c35d",
            tamanho: 93_939,
        },
    ],
};

/// Segmentação pyannote 3.0 (MIT, CNRS) e embeddings CAM++ do 3D-Speaker (Apache-2.0).
pub const FALANTES: ConjuntoDeModelo = ConjuntoDeModelo {
    pasta: "falantes",
    licenca: "MIT (pyannote/segmentation-3.0) e Apache-2.0 (3D-Speaker CAM++ VoxCeleb)",
    arquivos: &[
        ArquivoRemoto {
            arquivo: "segmentation.onnx",
            url: "https://huggingface.co/csukuangfj/sherpa-onnx-pyannote-segmentation-3-0/resolve/9403a6902bb58e3d5ae8c7e77c3422de279db2e0/model.onnx",
            sha256: "220ad67ca923bef2fa91f2390c786097bf305bceb5e261d4af67b38e938e1079",
            tamanho: 5_992_913,
        },
        ArquivoRemoto {
            arquivo: "3dspeaker_speech_campplus_sv_en_voxceleb_16k.onnx",
            url: "https://huggingface.co/csukuangfj/speaker-embedding-models/resolve/a8055d6daef723a6a4ecf4de233708575bf4994f/3dspeaker_speech_campplus_sv_en_voxceleb_16k.onnx",
            sha256: "98daf9bcf1c8abf3db374fec4c27a5eda7c58705bb3f180d057b085a72a88bb5",
            tamanho: 29_596_979,
        },
    ],
};

/// Mesmo GGUF do Mac (`ModeloRemoto.qwenPadrao`).
pub const QWEN: ConjuntoDeModelo = ConjuntoDeModelo {
    pasta: "",
    licenca: "Apache-2.0 (Qwen/Qwen3.5-2B; GGUF da Unsloth)",
    arquivos: &[ArquivoRemoto {
        arquivo: "Qwen3.5-2B-Q4_K_M.gguf",
        url: "https://huggingface.co/unsloth/Qwen3.5-2B-GGUF/resolve/f6d5376be1edb4d416d56da11e5397a961aca8ae/Qwen3.5-2B-Q4_K_M.gguf",
        sha256: "aaf42c8b7c3cab2bf3d69c355048d4a0ee9973d48f16c731c0520ee914699223",
        tamanho: 1_280_835_840,
    }],
};

/// llama.cpp oficial para Windows x64 (CPU), mesma versão do binário embutido no Mac (b12359).
/// Não é download do usuário: o empacotamento extrai daqui os arquivos de
/// [`LLAMA_WINDOWS_ARQUIVOS`] para a pasta `llama\` do instalador.
pub const LLAMA_WINDOWS_ZIP: ArquivoRemoto = ArquivoRemoto {
    arquivo: "llama-b12359-bin-win-cpu-x64.zip",
    url: "https://github.com/ggml-org/llama.cpp/releases/download/b12359/llama-b12359-bin-win-cpu-x64.zip",
    sha256: "837c1655e87df3b66989f4be33a792a1399c785bf259038dc75c900096683e69",
    tamanho: 25_097_509,
};

/// O que o instalador leva do zip acima (o resto são ferramentas que o app não usa).
/// Os `ggml-cpu-*.dll` vão todos: o ggml escolhe em tempo de execução o melhor para a CPU.
pub const LLAMA_WINDOWS_ARQUIVOS: &[&str] = &[
    "llama-server.exe",
    "llama-server-impl.dll",
    "llama-common.dll",
    "llama.dll",
    "mtmd.dll",
    "ggml.dll",
    "ggml-base.dll",
    "ggml-rpc.dll",
    "ggml-cpu-*.dll",
    "libomp.dll",
    "LICENSE-LLVM-OpenMP",
];

pub const MODELOS_DO_APP: &[ConjuntoDeModelo] = &[PARAKEET, FALANTES, QWEN];

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// SHA-256 de um arquivo, em hexadecimal minúsculo.
pub fn sha256_arquivo(caminho: &Path) -> Result<String, Erro> {
    let mut f = std::fs::File::open(caminho)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex(&h.finalize()))
}

fn agente_https() -> Result<ureq::Agent, Erro> {
    let b = ureq::AgentBuilder::new().timeout_connect(std::time::Duration::from_secs(20));
    #[cfg(windows)]
    let b = b.tls_connector(std::sync::Arc::new(native_tls::TlsConnector::new()?));
    Ok(b.build())
}

/// Baixa `a` para `pasta/<arquivo>`: grava em `.part`, retoma com Range, confere tamanho e
/// SHA-256 e só então renomeia. Arquivo final já presente e conferido não é baixado de novo.
/// `progresso(baixados, total)`.
pub fn baixar(
    a: &ArquivoRemoto,
    pasta: &Path,
    progresso: &mut dyn FnMut(u64, u64),
) -> Result<PathBuf, Erro> {
    std::fs::create_dir_all(pasta)?;
    let destino = pasta.join(a.arquivo);
    let marcador = pasta.join(format!("{}.conferido", a.arquivo));
    if std::fs::metadata(&destino)
        .map(|m| m.len() == a.tamanho)
        .unwrap_or(false)
    {
        let ja = std::fs::read_to_string(&marcador)
            .map(|s| s.trim() == a.sha256)
            .unwrap_or(false);
        if ja || sha256_arquivo(&destino)? == a.sha256 {
            std::fs::write(&marcador, a.sha256)?;
            progresso(a.tamanho, a.tamanho);
            return Ok(destino);
        }
    }
    let parcial = pasta.join(format!("{}.part", a.arquivo));
    let mut inicio = std::fs::metadata(&parcial).map(|m| m.len()).unwrap_or(0);
    if inicio > a.tamanho {
        std::fs::remove_file(&parcial)?;
        inicio = 0;
    }
    if inicio < a.tamanho {
        let agente = agente_https()?;
        let mut req = agente.get(a.url);
        if inicio > 0 {
            req = req.set("Range", &format!("bytes={inicio}-"));
        }
        let resp = req
            .call()
            .map_err(|e| erro(format!("download de {}: {e}", a.arquivo)))?;
        let mut arq = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&parcial)?;
        if resp.status() == 200 && inicio > 0 {
            arq.set_len(0)?; // servidor ignorou o Range: recomeça
            inicio = 0;
        }
        let mut leitor = resp.into_reader();
        let mut buf = vec![0u8; 1 << 16];
        let mut feitos = inicio;
        loop {
            let n = leitor.read(&mut buf)?;
            if n == 0 {
                break;
            }
            arq.write_all(&buf[..n])?;
            feitos += n as u64;
            progresso(feitos, a.tamanho);
        }
        arq.flush()?;
    }
    let tam = std::fs::metadata(&parcial)?.len();
    if tam != a.tamanho {
        return Err(erro(format!(
            "{}: {tam} bytes, esperado {}; tente de novo para retomar",
            a.arquivo, a.tamanho
        )));
    }
    let obtido = sha256_arquivo(&parcial)?;
    if obtido != a.sha256 {
        std::fs::remove_file(&parcial)?;
        return Err(erro(format!(
            "{}: SHA-256 {obtido} não confere com {}; descartado",
            a.arquivo, a.sha256
        )));
    }
    std::fs::rename(&parcial, &destino)?;
    std::fs::write(&marcador, a.sha256)?;
    Ok(destino)
}

/// Baixa todos os arquivos de um conjunto para `modelos/<pasta>`.
pub fn baixar_conjunto(
    c: &ConjuntoDeModelo,
    modelos: &Path,
    progresso: &mut dyn FnMut(&str, u64, u64),
) -> Result<PathBuf, Erro> {
    let destino = c.caminho(modelos);
    for a in c.arquivos {
        baixar(a, &destino, &mut |f, t| progresso(a.arquivo, f, t))?;
    }
    Ok(destino)
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn catalogo_bem_formado() {
        for c in MODELOS_DO_APP {
            for a in c.arquivos {
                assert_eq!(a.sha256.len(), 64, "{}", a.arquivo);
                assert!(a
                    .sha256
                    .chars()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
                assert!(a.url.starts_with("https://"));
                assert!(
                    a.url.ends_with(a.arquivo) || a.arquivo == "segmentation.onnx",
                    "{}",
                    a.url
                );
                // Fixado em revisão (hash de commit de 40 caracteres) ou em tag de release.
                assert!(
                    a.url
                        .split('/')
                        .any(|p| p.len() == 40 && p.chars().all(|c| c.is_ascii_hexdigit())),
                    "{}",
                    a.url
                );
                assert!(a.tamanho > 0);
            }
        }
        assert!(PARAKEET.tamanho() > 600_000_000);
        assert!(LLAMA_WINDOWS_ZIP.url.contains("/b12359/"));
    }

    #[test]
    fn baixar_reaproveita_arquivo_conferido_e_rejeita_hash_errado() {
        let pasta = std::env::temp_dir().join(format!("motores-cat-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&pasta);
        std::fs::create_dir_all(&pasta).unwrap();
        std::fs::write(pasta.join("x.bin"), b"abc").unwrap();
        let bom = ArquivoRemoto {
            arquivo: "x.bin",
            url: "https://invalido.example/x.bin",
            sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            tamanho: 3,
        };
        assert_eq!(
            baixar(&bom, &pasta, &mut |_, _| {}).unwrap(),
            pasta.join("x.bin")
        );
        assert!(pasta.join("x.bin.conferido").exists());
        // Parcial completo com hash errado: descartado, sem arquivo final.
        std::fs::write(pasta.join("y.bin.part"), b"abd").unwrap();
        let ruim = ArquivoRemoto {
            arquivo: "y.bin",
            ..bom
        };
        assert!(baixar(&ruim, &pasta, &mut |_, _| {}).is_err());
        assert!(!pasta.join("y.bin").exists() && !pasta.join("y.bin.part").exists());
        let _ = std::fs::remove_dir_all(&pasta);
    }
}
