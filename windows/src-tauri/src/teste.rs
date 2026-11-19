//! Modos de teste por linha de comando, para a sessão no PC Windows conferir a camada nativa
//! antes do núcleo estar integrado. O resultado vai para o registro e para um JSON na pasta do app
//! (só números: duração, nível, tempo gasto; nenhum conteúdo).
//!
//! `Dita.se.exe --testar-captura 10`  grava 10 s de microfone e de áudio do sistema (loopback)
//! `Dita.se.exe --testar-arquivo C:\caminho\video.mp4`  decodifica para 16 kHz mono e mede
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use serde_json::{json, Value};

use crate::{audio::{self, Captura, EscritorWav, Fonte}, caminhos, decodificacao};

/// Devolve `true` se um modo de teste rodou (o app então sai sem abrir janela).
pub fn rodar(args: &[String]) -> bool {
    let valor = |nome: &str| args.iter().position(|a| a == nome).and_then(|i| args.get(i + 1)).cloned();
    let resultado = if let Some(s) = valor("--testar-captura") {
        Some(("teste-captura.json", captura(s.parse().unwrap_or(10.0f64).clamp(1.0, 120.0))))
    } else {
        valor("--testar-arquivo").map(|c| ("teste-arquivo.json", arquivo(std::path::Path::new(&c))))
    };
    let Some((nome, valor)) = resultado else { return false };
    let texto = serde_json::to_string_pretty(&valor).unwrap_or_default();
    caminhos::nota(format!("teste: {}", valor));
    let _ = std::fs::write(caminhos::raiz().join(nome), &texto);
    println!("{texto}");
    true
}

#[derive(Default)]
struct Medida { amostras: usize, soma: f64, blocos: usize, maximo: f32 }

fn captura(segundos: f64) -> Value {
    let pasta = caminhos::audio_de_reuniao();
    let mut fontes = serde_json::Map::new();
    let mut abertas = Vec::new();
    for (fonte, nome) in [(Fonte::Microfone, "microfone"), (Fonte::Sistema, "sistema")] {
        let medida = Arc::new(Mutex::new(Medida::default()));
        let escritor = Arc::new(Mutex::new(EscritorWav::criar(&pasta.join(format!("teste-{nome}.wav"))).ok()));
        let (m, e) = (medida.clone(), escritor.clone());
        match Captura::iniciar(fonte, "", move |bloco, nivel| {
            let mut x = m.lock().unwrap();
            x.amostras += bloco.len(); x.soma += nivel as f64; x.blocos += 1; x.maximo = x.maximo.max(nivel);
            if let Some(w) = e.lock().unwrap().as_mut() { let _ = w.gravar(&bloco); }
        }) {
            Ok(c) => abertas.push((nome, c, medida)),
            Err(e) => { fontes.insert(nome.into(), json!({"erro": e.to_string()})); }
        }
    }
    std::thread::sleep(Duration::from_secs_f64(segundos));
    for (nome, mut c, m) in abertas {
        c.parar();
        let x = m.lock().unwrap();
        fontes.insert(nome.into(), json!({
            "segundos": x.amostras as f64 / audio::TAXA as f64,
            "nivelMedio": if x.blocos > 0 { x.soma / x.blocos as f64 } else { 0.0 },
            "nivelMaximo": x.maximo,
            "arquivo": pasta.join(format!("teste-{nome}.wav")).to_string_lossy(),
        }));
    }
    json!({"pedido": segundos, "fontes": fontes})
}

fn arquivo(caminho: &std::path::Path) -> Value {
    let inicio = Instant::now();
    match decodificacao::decodificar(caminho) {
        Ok(pcm) => json!({
            "extensao": decodificacao::extensao(caminho),
            "segundosDeAudio": pcm.len() as f64 / audio::TAXA as f64,
            "segundosGastos": inicio.elapsed().as_secs_f64(),
            "nivel": audio::nivel(&pcm),
        }),
        Err(e) => json!({"extensao": decodificacao::extensao(caminho), "erro": e.to_string()}),
    }
}
