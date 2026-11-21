//! Medição dos motores sobre um corpus de WAVs com transcrição de referência.
//!
//! Uso:
//!   medir --corpus motores/corpus [--modelos DIR] [--baixar] [--threads N]
//!         [--falantes [--detalhe]] [--limiar N] [--llama BIN --gguf ARQ --llama-threads N --contexto N] [--json SAIDA.json]
//!   medir --diagnostico-llama --llama BIN --gguf ARQ [--sha256 HEX]
//!
//! `--diagnostico-llama` sobe o `llama-server` do pacote com um GGUF público minúsculo e faz uma
//! completação pelo canal TLS autenticado de produção. Termina com código 0 só se tudo passar.
//!
//! Corpus: `<nome>.wav` (qualquer taxa, mono ou estéreo) com `<nome>.txt` ao lado (referência).
//! Falantes: `falantes/<nome>.wav` com `falantes/<nome>.json` = [{"voz","inicio","fim"}, ...].
//! Mede: tempo de carga, RTF (tempo de processamento / duração do áudio), RAM de pico do
//! processo e WER (minúsculas, sem pontuação, acentos contam). Com `--falantes`, número de
//! vozes e erro de atribuição por quadro de 10 ms. Com `--llama`, latência da limpeza.
use motores::catalogo::{self, FALANTES, PARAKEET};
use motores::fala::Parakeet;
use motores::falantes::{Diarizador, LIMIAR_PADRAO};
use motores::llama::{Ajustes, ServidorLlama, PROMPT_LIMPEZA};
use motores::medicao::{erros_de_palavras, ram_de_pico};
use motores::{
    audio, FaixaDeVoz, IdentificadorDeFalantes, Limpador, PedidoLimpeza, Transcritor, TAXA,
};
use std::path::{Path, PathBuf};
use std::time::Instant;

fn mb(b: u64) -> f64 {
    b as f64 / (1024.0 * 1024.0)
}

struct Args {
    corpus: PathBuf,
    modelos: PathBuf,
    baixar: bool,
    beam: bool,
    threads: usize,
    falantes: bool,
    limiar: f32,
    ajustes: Ajustes,
    llama: Option<PathBuf>,
    gguf: Option<PathBuf>,
    json: Option<PathBuf>,
    fala_json: Option<PathBuf>,
    diagnostico: bool,
    detalhe: bool,
    sha256: Option<String>,
}

fn args() -> Args {
    let mut a = Args {
        corpus: PathBuf::from("motores/corpus"),
        modelos: motores::pasta_de_teste(),
        baixar: false,
        beam: false,
        threads: 0,
        falantes: false,
        limiar: LIMIAR_PADRAO,
        ajustes: Ajustes::default(),
        llama: None,
        gguf: None,
        json: None,
        fala_json: None,
        diagnostico: false,
        detalhe: false,
        sha256: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(x) = it.next() {
        let mut v = || it.next().unwrap_or_else(|| panic!("{x} pede um valor"));
        match x.as_str() {
            "--corpus" => a.corpus = v().into(),
            "--modelos" => a.modelos = v().into(),
            "--threads" => a.threads = v().parse().expect("--threads N"),
            "--limiar" => a.limiar = v().parse().expect("--limiar N"),
            "--llama-threads" => a.ajustes.threads = Some(v().parse().expect("--llama-threads N")),
            "--contexto" => a.ajustes.contexto = v().parse().expect("--contexto N"),
            "--llama" => a.llama = Some(v().into()),
            "--gguf" => a.gguf = Some(v().into()),
            "--fala-json" => a.fala_json = Some(v().into()),
            "--json" => a.json = Some(v().into()),
            "--beam" => a.beam = true,
            "--baixar" => a.baixar = true,
            "--falantes" => a.falantes = true,
            "--diagnostico-llama" => a.diagnostico = true,
            "--detalhe" => a.detalhe = true,
            "--sha256" => a.sha256 = Some(v().to_ascii_lowercase()),
            outro => panic!("opção desconhecida: {outro}"),
        }
    }
    a
}

fn pares(pasta: &Path, extensao: &str) -> Vec<(PathBuf, PathBuf)> {
    let mut v: Vec<_> = std::fs::read_dir(pasta)
        .map(|d| {
            d.filter_map(Result::ok)
                .map(|e| e.path())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
        .into_iter()
        .filter(|p| p.extension().is_some_and(|e| e == "wav"))
        .filter_map(|w| {
            let r = w.with_extension(extensao);
            r.is_file().then_some((w, r))
        })
        .collect();
    v.sort();
    v
}

/// Erro de atribuição por quadro de 10 ms, com o melhor casamento entre vozes (força bruta,
/// poucas vozes). Fala perdida, fala inventada e voz trocada contam como erro.
fn erro_de_diarizacao(refe: &[FaixaDeVoz], hip: &[FaixaDeVoz], duracao: f64) -> f64 {
    let quadros = (duracao * 100.0).ceil() as usize;
    let rotulos = |fs: &[FaixaDeVoz]| {
        let mut nomes: Vec<String> = fs.iter().map(|f| f.voz.clone()).collect();
        nomes.sort();
        nomes.dedup();
        let mut q = vec![None; quadros];
        for f in fs {
            let k = nomes.iter().position(|n| *n == f.voz).unwrap();
            let de = ((f.inicio * 100.0) as usize).min(quadros);
            let ate = ((f.fim * 100.0) as usize).min(quadros);
            q[de..ate.max(de)].fill(Some(k));
        }
        (q, nomes.len())
    };
    let (qr, nr) = rotulos(refe);
    let (qh, nh) = rotulos(hip);
    let fala = qr.iter().filter(|x| x.is_some()).count().max(1);
    let mut melhor = usize::MAX;
    let mut perm: Vec<usize> = (0..nh.max(nr)).collect();
    permutar(&mut perm, 0, &mut |p| {
        let erros = qr.iter().zip(&qh).filter(|(r, h)| match (r, h) {
            (None, None) => false,
            (Some(r), Some(h)) => p[*h] != *r,
            _ => true,
        });
        melhor = melhor.min(erros.count());
    });
    melhor as f64 / fala as f64
}

fn permutar(v: &mut Vec<usize>, k: usize, f: &mut dyn FnMut(&[usize])) {
    if k >= v.len() {
        return f(v);
    }
    for i in k..v.len() {
        v.swap(k, i);
        permutar(v, k + 1, f);
        v.swap(k, i);
    }
}

fn main() {
    let a = args();
    if a.diagnostico {
        let (Some(bin), Some(gguf)) = (&a.llama, &a.gguf) else {
            eprintln!("--diagnostico-llama precisa de --llama BIN e --gguf ARQ");
            std::process::exit(2);
        };
        match diagnosticar_llama(bin, gguf, a.sha256.as_deref()) {
            Ok(()) => println!("DIAGNOSTICO LLAMA OK"),
            Err(e) => {
                println!("DIAGNOSTICO LLAMA FALHOU: {e}");
                std::process::exit(1);
            }
        }
        return;
    }
    let mut rel = serde_json::Map::new();
    rel.insert(
        "plataforma".into(),
        format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH).into(),
    );
    rel.insert(
        "threads_cpu".into(),
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(0)
            .into(),
    );

    // Reutiliza exatamente as mesmas hipóteses para isolar a latência e a WER do Qwen.
    if let Some(arquivo) = &a.fala_json {
        assert!(
            a.llama.is_some() && a.gguf.is_some(),
            "--fala-json precisa de --llama e --gguf"
        );
        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(arquivo).expect("ler JSON"))
                .expect("JSON de fala");
        let itens = json["fala"]["itens"]
            .as_array()
            .expect("relatório sem fala.itens");
        assert!(!itens.is_empty(), "relatório sem amostras");
        let frases = itens
            .iter()
            .map(|item| {
                (
                    item["texto"].as_str().expect("texto").to_owned(),
                    item["referencia"].as_str().expect("referência").to_owned(),
                )
            })
            .collect::<Vec<_>>();
        medir_limpeza(&a, &frases, &mut rel);
        salvar(&a, rel);
        return;
    }

    if a.baixar {
        for c in [&PARAKEET, &FALANTES] {
            catalogo::baixar_conjunto(c, &a.modelos, &mut |arq, f, t| {
                if f == t {
                    eprintln!("ok {arq}");
                }
            })
            .expect("download");
        }
    }

    // --- Fala
    let ram_inicial = ram_de_pico();
    let t0 = Instant::now();
    let parakeet = Parakeet::carregar_com_busca(&PARAKEET.caminho(&a.modelos), a.threads, a.beam)
        .expect("carregar Parakeet");
    let carga = t0.elapsed().as_secs_f64();
    let corpus = pares(&a.corpus, "txt");
    assert!(
        !corpus.is_empty(),
        "nenhum par .wav/.txt em {}",
        a.corpus.display()
    );
    println!("Parakeet TDT 0.6B v3 INT8 (sherpa-onnx, CPU): carga {carga:.2} s");
    println!(
        "{:<28} {:>7} {:>7} {:>6} {:>6}  transcrição",
        "arquivo", "áudio s", "proc s", "RTF", "WER"
    );
    let (mut erros, mut total, mut audio_s, mut proc_s) = (0, 0, 0.0, 0.0);
    let mut itens = Vec::new();
    let mut frases = Vec::new();
    for (wav, txt) in &corpus {
        let amostras = audio::ler_wav(wav).expect("ler wav");
        let referencia = std::fs::read_to_string(txt).unwrap();
        let dur = amostras.len() as f64 / TAXA as f64;
        let t = Instant::now();
        let texto = parakeet.transcrever(&amostras).expect("transcrever");
        let p = t.elapsed().as_secs_f64();
        let (e, n) = erros_de_palavras(&referencia, &texto);
        let nome = wav.file_stem().unwrap().to_string_lossy().to_string();
        println!(
            "{:<28} {:>7.2} {:>7.2} {:>6.3} {:>5.1}%  {}",
            nome,
            dur,
            p,
            p / dur,
            100.0 * e as f64 / n as f64,
            texto
        );
        erros += e;
        total += n;
        audio_s += dur;
        proc_s += p;
        frases.push((texto.clone(), referencia.clone()));
        itens.push(serde_json::json!({"arquivo": nome, "audio_s": dur, "proc_s": p, "erros": e, "palavras": n, "texto": texto, "referencia": referencia.trim()}));
    }
    let ram_fala = ram_de_pico();
    let wer = 100.0 * erros as f64 / total.max(1) as f64;
    println!(
        "TOTAL: {} arquivos, {audio_s:.1} s de áudio, RTF {:.3}, WER {wer:.2}% ({erros}/{total}), RAM de pico {:.0} MB (antes da carga {:.0} MB)",
        corpus.len(),
        proc_s / audio_s,
        mb(ram_fala),
        mb(ram_inicial)
    );
    rel.insert(
        "fala".into(),
        serde_json::json!({"modelo": "parakeet-tdt-0.6b-v3-int8", "carga_s": carga, "audio_s": audio_s, "proc_s": proc_s,
            "rtf": proc_s / audio_s, "wer_pct": wer, "erros": erros, "palavras": total, "ram_pico_mb": mb(ram_fala), "itens": itens}),
    );

    // --- Falantes
    if a.falantes {
        let t0 = Instant::now();
        let diar = Diarizador::carregar(&FALANTES.caminho(&a.modelos), None, a.limiar, a.threads)
            .expect("carregar diarização");
        let carga = t0.elapsed().as_secs_f64();
        println!(
            "\nDiarização (pyannote 3.0 + CAM++, sherpa-onnx): carga {carga:.2} s, limiar {}",
            a.limiar
        );
        let mut itens = Vec::new();
        for (wav, js) in pares(&a.corpus.join("falantes"), "json") {
            let refe: Vec<FaixaDeVoz> = serde_json::from_str::<Vec<serde_json::Value>>(
                &std::fs::read_to_string(&js).unwrap(),
            )
            .unwrap()
            .iter()
            .map(|v| FaixaDeVoz {
                voz: v["voz"].as_str().unwrap().into(),
                inicio: v["inicio"].as_f64().unwrap(),
                fim: v["fim"].as_f64().unwrap(),
            })
            .collect();
            let dur = audio::ler_wav(&wav).unwrap().len() as f64 / TAXA as f64;
            let t = Instant::now();
            let hip = diar.faixas(&wav).expect("diarizar");
            let p = t.elapsed().as_secs_f64();
            let mut vr: Vec<_> = refe.iter().map(|f| &f.voz).collect();
            vr.sort();
            vr.dedup();
            let mut vh: Vec<_> = hip.iter().map(|f| &f.voz).collect();
            vh.sort();
            vh.dedup();
            let der = erro_de_diarizacao(&refe, &hip, dur);
            let nome = wav.file_stem().unwrap().to_string_lossy().to_string();
            println!(
                "{nome:<28} áudio {dur:.1} s, proc {p:.2} s (RTF {:.3}), vozes {} de {}, erro por quadro {:.1}%",
                p / dur,
                vh.len(),
                vr.len(),
                der * 100.0
            );
            if a.detalhe {
                for f in &refe {
                    println!(
                        "    referência {:>6.2} a {:>6.2}  {}",
                        f.inicio, f.fim, f.voz
                    );
                }
                for f in &hip {
                    println!(
                        "    obtido     {:>6.2} a {:>6.2}  {}",
                        f.inicio, f.fim, f.voz
                    );
                }
            }
            itens.push(serde_json::json!({"arquivo": nome, "audio_s": dur, "proc_s": p, "vozes_ref": vr.len(), "vozes_obtidas": vh.len(), "erro_quadro": der}));
        }
        rel.insert(
            "falantes".into(),
            serde_json::json!({"carga_s": carga, "ram_pico_mb": mb(ram_de_pico()), "itens": itens}),
        );
        println!("RAM de pico com diarização: {:.0} MB", mb(ram_de_pico()));
    }

    medir_limpeza(&a, &frases, &mut rel);
    salvar(&a, rel);
}

/// Prova, no binário que vai no instalador, que o desenho TLS + chave funciona: o servidor
/// conhece as opções, sobe com a identidade da partida numa pasta com acento, exige a chave,
/// completa um pedido e não deixa a chave privada no disco.
fn diagnosticar_llama(bin: &Path, gguf: &Path, sha256: Option<&str>) -> Result<(), String> {
    use sha2::{Digest, Sha256};
    let etapa = |nome: &str| println!("ok  {nome}");
    if let Some(esperado) = sha256 {
        let bytes = std::fs::read(gguf).map_err(|e| format!("ler GGUF: {e}"))?;
        let obtido: String = Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        if obtido != esperado {
            return Err(format!("SHA-256 do GGUF {obtido}, esperado {esperado}"));
        }
        etapa("SHA-256 do GGUF");
    }
    let mut ajuda = std::process::Command::new(bin);
    ajuda.arg("--help");
    if let Some(pasta) = bin.parent().filter(|p| !p.as_os_str().is_empty()) {
        ajuda.current_dir(pasta);
    }
    let saida = ajuda
        .output()
        .map_err(|e| format!("executar {} --help: {e}", bin.display()))?;
    let texto = format!(
        "{}{}",
        String::from_utf8_lossy(&saida.stdout),
        String::from_utf8_lossy(&saida.stderr)
    );
    for opcao in ["--ssl-key-file", "--ssl-cert-file", "--api-key"] {
        if !texto.contains(opcao) {
            return Err(format!(
                "--help sem {opcao} (status {}): binário sem suporte",
                saida.status
            ));
        }
    }
    etapa("--help lista --ssl-key-file, --ssl-cert-file e --api-key");

    let raiz = std::env::temp_dir().join(format!("ditase-diagnóstico-{}", std::process::id()));
    let pasta = raiz.join("Configurações çã");
    std::fs::create_dir_all(&pasta).map_err(|e| format!("pasta de trabalho: {e}"))?;
    let resultado = (|| {
        let ajustes = Ajustes {
            contexto: 256,
            prazo_de_subida: std::time::Duration::from_secs(60),
            threads: Some(2),
            pasta_de_trabalho: Some(pasta.clone()),
            ..Ajustes::default()
        };
        let servidor = ServidorLlama::novo(bin, PROMPT_LIMPEZA, ajustes);
        let t = Instant::now();
        let porta = servidor
            .garantir(gguf)
            .map_err(|e| format!("subida: {e}"))?;
        etapa(&format!(
            "subida TLS em 127.0.0.1:{porta}, 401 sem chave, 200 com chave ({:.2} s)",
            t.elapsed().as_secs_f64()
        ));
        let restos = std::fs::read_dir(&pasta)
            .map_err(|e| format!("listar pasta: {e}"))?
            .count();
        if restos != 0 {
            return Err(format!(
                "{restos} arquivo(s) TLS ficaram na pasta de trabalho"
            ));
        }
        etapa("chave e certificado apagados depois da subida");
        let corpo = serde_json::json!({
            "messages": [{"role": "user", "content": "Once upon a time"}],
            "max_tokens": 16,
            "temperature": 0,
        });
        let t = Instant::now();
        let json = servidor
            .completar(gguf, &corpo, std::time::Duration::from_secs(60))
            .map_err(|e| format!("completação: {e}"))?;
        let tokens = json["usage"]["completion_tokens"].as_u64().unwrap_or(0);
        if json["choices"][0]["message"].is_null() || tokens == 0 {
            return Err("completação sem choices[0].message ou sem tokens".into());
        }
        etapa(&format!(
            "completação pelo canal autenticado: {tokens} token(s) em {:.2} s",
            t.elapsed().as_secs_f64()
        ));
        let pid = servidor.pid();
        servidor.parar();
        etapa(&format!("servidor parado (pid {pid:?})"));
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&raiz);
    resultado
}

fn medir_limpeza(
    a: &Args,
    frases: &[(String, String)],
    rel: &mut serde_json::Map<String, serde_json::Value>,
) {
    // --- Limpeza
    if let (Some(bin), Some(gguf)) = (&a.llama, &a.gguf) {
        let servidor = ServidorLlama::novo(bin, PROMPT_LIMPEZA, a.ajustes.clone());
        let t0 = Instant::now();
        servidor.garantir(gguf).expect("subir llama-server");
        let subida = t0.elapsed().as_secs_f64();
        let limpador = servidor.limpador(gguf);
        println!(
            "\nLimpeza ({}, llama-server): subida autenticada {subida:.2} s",
            limpador.nome()
        );
        let mut tempos = Vec::new();
        let mut itens = Vec::new();
        let (mut erros_l, mut total_l) = (0, 0);
        for (f, referencia) in frases {
            let t = Instant::now();
            let r = limpador.limpar(&PedidoLimpeza {
                transcricao: f.clone(),
                ..Default::default()
            });
            let p = t.elapsed().as_secs_f64();
            tempos.push(p);
            let saida = r.expect("limpeza falhou, medição inválida");
            let (e, n) = erros_de_palavras(referencia, &saida);
            erros_l += e;
            total_l += n;
            println!("{p:>6.2} s  {saida}");
            itens.push(serde_json::json!({"entrada": f, "saida": saida, "s": p}));
        }
        servidor.parar();
        let primeira = tempos.first().copied().unwrap_or(0.0);
        tempos.sort_by(f64::total_cmp);
        let mediana = tempos.get(tempos.len() / 2).copied().unwrap_or(0.0);
        let ram_filho = ram_de_pico_filhos();
        println!(
            "Limpeza: mediana {mediana:.2} s, máximo {:.2} s, WER depois da limpeza {:.2}%, RAM de pico do llama-server {}",
            tempos.last().copied().unwrap_or(0.0),
            100.0 * erros_l as f64 / total_l.max(1) as f64,
            ram_filho.map(|b| format!("{:.0} MB", mb(b))).unwrap_or_else(|| "n/d".into())
        );
        rel.insert(
            "limpeza".into(),
            serde_json::json!({"subida_s": subida, "primeira_s": primeira, "mediana_s": mediana, "wer_pct": 100.0 * erros_l as f64 / total_l.max(1) as f64, "ram_pico_servidor_mb": ram_filho.map(mb), "itens": itens}),
        );
    }
}

fn salvar(a: &Args, rel: serde_json::Map<String, serde_json::Value>) {
    if let Some(j) = &a.json {
        std::fs::write(
            j,
            serde_json::to_string_pretty(&serde_json::Value::Object(rel)).unwrap(),
        )
        .expect("gravar json");
        println!("\nrelatório: {}", j.display());
    }
}

/// RAM de pico dos processos filhos já encerrados (llama-server). Só Unix.
fn ram_de_pico_filhos() -> Option<u64> {
    #[cfg(unix)]
    unsafe {
        let mut u: libc::rusage = std::mem::zeroed();
        libc::getrusage(libc::RUSAGE_CHILDREN, &mut u);
        let b = if cfg!(target_os = "macos") {
            u.ru_maxrss as u64
        } else {
            u.ru_maxrss as u64 * 1024
        };
        return (b > 0).then_some(b);
    }
    #[allow(unreachable_code)]
    None
}
