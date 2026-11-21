//! Modos de diagnóstico por linha de comando, para a sessão no PC Windows conferir a camada nativa.
//! Por padrão medem sem gravar nada: o resultado tem só os campos aprovados abaixo (duração, nível,
//! tempo gasto, categoria de erro), sem caminho nem mensagem crua.
//!
//! `Dita.se.exe --testar-captura 10`  mede 10 s de microfone e de áudio do sistema (loopback)
//! `Dita.se.exe --testar-captura 10 --guardar-wav`  idem e guarda os WAVs na pasta de trabalho
//!   (ação explícita; o resultado só diz que guardou, sem o caminho)
//! `Dita.se.exe --testar-arquivo C:\caminho\video.mp4`  decodifica para 16 kHz mono e mede
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use serde_json::{json, Map, Value};

use crate::{
    audio::{self, Captura, EscritorWav, Fonte},
    caminhos, decodificacao,
};

/// Únicos campos que um resultado de diagnóstico pode ter.
pub const CAMPOS_APROVADOS: &[&str] = &[
    "pedido",
    "fontes",
    "microfone",
    "sistema",
    "segundos",
    "nivelMedio",
    "nivelMaximo",
    "guardado",
    "extensao",
    "segundosDeAudio",
    "segundosGastos",
    "nivel",
    "erro",
];

/// Devolve `true` se um modo de diagnóstico rodou (o app então sai sem abrir janela).
pub fn rodar(args: &[String]) -> bool {
    let valor = |nome: &str| {
        args.iter()
            .position(|a| a == nome)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let modo_captura = valor("--testar-captura");
    let modo_arquivo = valor("--testar-arquivo");
    if modo_captura.is_none() && modo_arquivo.is_none() {
        return false;
    }
    if let Err(e) = caminhos::preparar() {
        eprintln!("{e}");
        std::process::exit(1);
    }
    let guardar = args.iter().any(|a| a == "--guardar-wav");
    let (nome, valor) = if let Some(s) = modo_captura {
        (
            "teste-captura.json",
            captura(s.parse().unwrap_or(10.0f64).clamp(1.0, 120.0), guardar),
        )
    } else {
        let c = modo_arquivo.expect("conferido acima");
        ("teste-arquivo.json", arquivo(std::path::Path::new(&c)))
    };
    let valor = filtrar(valor);
    let texto = serde_json::to_string_pretty(&valor).unwrap_or_default();
    caminhos::nota(format!("teste: {valor}"));
    let _ = std::fs::write(caminhos::raiz().join(nome), &texto);
    println!("{texto}");
    true
}

/// Mantém só os campos de `CAMPOS_APROVADOS`, em qualquer nível.
pub fn filtrar(v: Value) -> Value {
    match v {
        Value::Object(m) => Value::Object(
            m.into_iter()
                .filter(|(k, _)| CAMPOS_APROVADOS.contains(&k.as_str()))
                .map(|(k, v)| (k, filtrar(v)))
                .collect::<Map<_, _>>(),
        ),
        outro => outro,
    }
}

/// Categoria do erro em vez da mensagem crua (que pode trazer caminho ou nome de dispositivo).
pub fn categoria(erro: &str) -> &'static str {
    let e = erro.to_lowercase();
    if e.contains("nenhum") || e.contains("not found") || e.contains("no device") {
        "semDispositivo"
    } else if e.contains("formato") || e.contains("format") || e.contains("unsupported") {
        "formatoNaoSuportado"
    } else if e.contains("acesso")
        || e.contains("access")
        || e.contains("denied")
        || e.contains("permiss")
    {
        "semPermissao"
    } else {
        "falhou"
    }
}

#[derive(Default)]
struct Medida {
    amostras: usize,
    soma: f64,
    blocos: usize,
    maximo: f32,
}

fn captura(segundos: f64, guardar: bool) -> Value {
    let pasta = caminhos::trabalho();
    let mut fontes = Map::new();
    let mut abertas = Vec::new();
    for (fonte, nome) in [(Fonte::Microfone, "microfone"), (Fonte::Sistema, "sistema")] {
        let medida = Arc::new(Mutex::new(Medida::default()));
        let escritor = Arc::new(Mutex::new(if guardar {
            EscritorWav::criar(&pasta.join(format!("teste-{nome}.wav"))).ok()
        } else {
            None
        }));
        let guardou = escritor.lock().unwrap().is_some();
        let (m, e) = (medida.clone(), escritor.clone());
        match Captura::iniciar(fonte, "", move |bloco, nivel| {
            let mut x = m.lock().unwrap();
            x.amostras += bloco.len();
            x.soma += nivel as f64;
            x.blocos += 1;
            x.maximo = x.maximo.max(nivel);
            if let Some(w) = e.lock().unwrap().as_mut() {
                let _ = w.gravar(&bloco);
            }
        }) {
            Ok(c) => abertas.push((nome, c, medida, guardou)),
            Err(e) => {
                fontes.insert(nome.into(), json!({"erro": categoria(&e.to_string())}));
            }
        }
    }
    std::thread::sleep(Duration::from_secs_f64(segundos));
    for (nome, mut c, m, guardou) in abertas {
        c.parar();
        let x = m.lock().unwrap();
        fontes.insert(
            nome.into(),
            json!({
                "segundos": x.amostras as f64 / audio::TAXA as f64,
                "nivelMedio": if x.blocos > 0 { x.soma / x.blocos as f64 } else { 0.0 },
                "nivelMaximo": x.maximo,
                "guardado": guardou,
            }),
        );
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
        Err(e) => {
            json!({"extensao": decodificacao::extensao(caminho), "erro": categoria(&e.to_string())})
        }
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn resultado_so_tem_campos_aprovados() {
        let v = filtrar(
            json!({"pedido": 10, "arquivo": "C:\\Users\\ana\\x.wav", "fontes": {"microfone": {"segundos": 1, "caminho": "x", "erro": "semDispositivo"}}}),
        );
        assert_eq!(
            v,
            json!({"pedido": 10, "fontes": {"microfone": {"segundos": 1, "erro": "semDispositivo"}}})
        );
    }

    #[test]
    fn erro_vira_categoria_sem_texto_cru() {
        assert_eq!(categoria("nenhum microfone encontrado"), "semDispositivo");
        assert_eq!(
            categoria(r"Acesso negado: C:\Users\ana\Music\x.mp4"),
            "semPermissao"
        );
        assert_eq!(categoria("algo estranho"), "falhou");
    }
}
