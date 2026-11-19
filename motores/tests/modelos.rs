//! Testes com os modelos reais. Desligados por padrão; ligue com variáveis:
//! - `DITASE_TESTES_MODELOS=1`: Parakeet e diarização (modelos em `DITASE_MODELOS` ou
//!   `/tmp/ditase-motores-modelos`; `medir --baixar` baixa) e llama-server
//!   (`DITASE_LLAMA_SERVER`, padrão `Recursos/llama/llama-server`; `DITASE_GGUF`, padrão
//!   `/tmp/ditase-modelo-prova/Qwen3.5-2B-Q4_K_M.gguf`).
//! - `DITASE_TESTES_REDE=1`: download real de um arquivo pequeno do catálogo.
use motores::catalogo::{self, FALANTES, PARAKEET};
use motores::fala::Parakeet;
use motores::falantes::{Diarizador, LIMIAR_PADRAO};
use motores::llama::{Ajustes, Estado, ServidorLlama, PROMPT_LIMPEZA};
use motores::medicao::erros_de_palavras;
use motores::{
    audio, IdentificadorDeFalantes, Limpador, PedidoLimpeza, ReconhecedorDeFala, Transcritor,
};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

fn ligado(var: &str) -> bool {
    let sim = std::env::var(var).map(|v| v == "1").unwrap_or(false);
    if !sim {
        eprintln!("pulado: defina {var}=1");
    }
    sim
}

fn raiz() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}
fn corpus(nome: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("corpus")
        .join(nome)
}

fn parakeet() -> &'static Parakeet {
    static P: OnceLock<Parakeet> = OnceLock::new();
    P.get_or_init(|| {
        Parakeet::carregar(&PARAKEET.caminho(&motores::pasta_de_teste()), 0)
            .expect("Parakeet: rode `medir --baixar`")
    })
}

#[test]
fn parakeet_transcreve_pt_br() {
    if !ligado("DITASE_TESTES_MODELOS") {
        return;
    }
    let amostras = audio::ler_wav(&corpus("luciana-01.wav")).unwrap();
    let texto = parakeet().transcrever(&amostras).unwrap();
    let referencia = std::fs::read_to_string(corpus("luciana-01.txt")).unwrap();
    let (e, n) = erros_de_palavras(&referencia, &texto);
    assert!(e * 10 <= n, "WER alto: {texto:?}");
    assert_eq!(parakeet().nome(), "parakeet-tdt-0.6b-v3");
    // Trecho curto (abaixo de 1,1 s) e silêncio não quebram.
    assert!(parakeet().transcrever(&amostras[..8000]).is_ok());
    assert!(
        parakeet()
            .transcrever(&[0.0; 100])
            .unwrap()
            .split_whitespace()
            .count()
            <= 1
    );
}

#[test]
fn palavras_com_tempo_absoluto_e_cauda_fechada() {
    if !ligado("DITASE_TESTES_MODELOS") {
        return;
    }
    let amostras = audio::ler_wav(&corpus("luciana-10.wav")).unwrap();
    let dur = amostras.len() as f64 / 16_000.0;
    let inicio = 42.0;
    let livres = parakeet().palavras(&amostras, inicio, None).unwrap();
    let texto: Vec<&str> = livres.iter().map(|p| p.texto.as_str()).collect();
    assert!(texto.len() >= 12, "{texto:?}");
    assert!(
        texto.last().unwrap().starts_with("grupo"),
        "a última palavra não pode sumir: {texto:?}"
    );
    for w in livres.windows(2) {
        assert!(w[0].inicio <= w[1].inicio, "fora de ordem: {w:?}");
    }
    assert!(livres
        .iter()
        .all(|p| p.inicio >= inicio && p.fim <= inicio + dur + 1e-9 && p.fim > p.inicio));

    // fim_da_fala no meio: nenhuma palavra passa dele.
    let corte = 2.0;
    let fechadas = parakeet().palavras(&amostras, inicio, Some(corte)).unwrap();
    assert!(!fechadas.is_empty());
    assert!(
        fechadas.iter().all(|p| p.fim <= inicio + corte + 1e-9),
        "{fechadas:?}"
    );

    // Silêncio puro: nada (regra do Swift, abaixo de 0,002 não reconhece).
    assert!(parakeet()
        .palavras(&vec![0.001; 32_000], 0.0, Some(2.0))
        .unwrap()
        .is_empty());
}

#[test]
fn diarizacao_separa_tres_vozes() {
    if !ligado("DITASE_TESTES_MODELOS") {
        return;
    }
    let d = Diarizador::carregar(
        &FALANTES.caminho(&motores::pasta_de_teste()),
        None,
        LIMIAR_PADRAO,
        0,
    )
    .unwrap();
    let faixas = d.faixas(&corpus("falantes/conversa-3-vozes.wav")).unwrap();
    let mut vozes: Vec<_> = faixas.iter().map(|f| f.voz.clone()).collect();
    vozes.sort();
    vozes.dedup();
    assert_eq!(vozes.len(), 3, "{faixas:?}");
    assert!(faixas
        .iter()
        .all(|f| f.voz.starts_with("offline-") && f.fim > f.inicio));
    // A última fala (voz A, depois de 28 s) aparece.
    assert!(faixas.iter().any(|f| f.fim > 30.0), "{faixas:?}");
    // Áudio curto demais: vazio, sem erro.
    assert!(d.faixas_de(&[0.0; 4000]).unwrap().is_empty());
}

fn llama() -> Option<(PathBuf, PathBuf)> {
    let bin = std::env::var_os("DITASE_LLAMA_SERVER")
        .map(PathBuf::from)
        .unwrap_or_else(|| raiz().join("Recursos/llama/llama-server"));
    let gguf = std::env::var_os("DITASE_GGUF")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp/ditase-modelo-prova/Qwen3.5-2B-Q4_K_M.gguf"));
    if bin.is_file() && gguf.is_file() {
        Some((bin, gguf))
    } else {
        eprintln!("pulado: llama-server ou GGUF ausente");
        None
    }
}

fn vivo(pid: u32) -> bool {
    #[cfg(unix)]
    {
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
    #[cfg(windows)]
    {
        let saida = std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&saida.stdout).contains(&pid.to_string())
    }
}

#[test]
fn llama_server_sobe_limpa_e_encerra() {
    if !ligado("DITASE_TESTES_MODELOS") {
        return;
    }
    let Some((bin, gguf)) = llama() else { return };
    let ajustes = Ajustes {
        ocioso: Duration::from_secs(3),
        ..Default::default()
    };
    let servidor = ServidorLlama::novo(&bin, PROMPT_LIMPEZA, ajustes);
    let limpador = servidor.limpador(&gguf);
    assert_eq!(limpador.nome(), "Qwen3.5-2B-Q4_K_M");

    // Dois pedidos simultâneos esperam a mesma subida.
    let l2 = servidor.limpador(&gguf);
    let pedido = PedidoLimpeza {
        transcricao: "reserva a sala na sexta, não, espera, reserva na quinta às duas que é melhor"
            .into(),
        ..Default::default()
    };
    let p2 = pedido.clone();
    let outro = std::thread::spawn(move || l2.limpar(&p2));
    let saida = limpador.limpar(&pedido).unwrap();
    let saida2 = outro.join().unwrap().unwrap();
    assert!(saida.to_lowercase().contains("quinta"), "{saida}");
    assert!(
        !saida.to_lowercase().contains("sexta"),
        "autocorreção não resolvida: {saida}"
    );
    assert_eq!(saida, saida2, "temperatura 0 deve dar a mesma saída");
    assert_eq!(servidor.estado(), Estado::Pronto);
    let pid = servidor.pid().unwrap();
    assert!(vivo(pid));

    // Ociosidade: desliga sozinho e devolve a memória.
    let prazo = Instant::now() + Duration::from_secs(15);
    while servidor.estado() != Estado::Desligado && Instant::now() < prazo {
        std::thread::sleep(Duration::from_millis(200));
    }
    assert_eq!(servidor.estado(), Estado::Desligado);
    std::thread::sleep(Duration::from_millis(300));
    assert!(!vivo(pid), "llama-server ficou órfão");

    // Sobe de novo sob demanda; Drop encerra o processo.
    let porta = servidor.garantir(&gguf).unwrap();
    assert!(porta > 0);
    let pid = servidor.pid().unwrap();
    drop(limpador);
    drop(servidor);
    std::thread::sleep(Duration::from_millis(300));
    assert!(!vivo(pid), "Drop não encerrou o llama-server");
}

#[test]
fn download_real_confere_hash() {
    if !ligado("DITASE_TESTES_REDE") {
        return;
    }
    let pasta = std::env::temp_dir().join(format!("motores-rede-{}", std::process::id()));
    let tokens = PARAKEET
        .arquivos
        .iter()
        .find(|a| a.arquivo == "tokens.txt")
        .unwrap();
    let mut ultimo = 0;
    let caminho = catalogo::baixar(tokens, &pasta, &mut |f, _| ultimo = f).unwrap();
    assert_eq!(ultimo, tokens.tamanho);
    assert_eq!(catalogo::sha256_arquivo(&caminho).unwrap(), tokens.sha256);
    let _ = std::fs::remove_dir_all(&pasta);
}
