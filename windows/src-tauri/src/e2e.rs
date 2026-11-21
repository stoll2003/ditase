//! Prova de ponta a ponta com os modelos reais (Parakeet, diarização e llama), pelo mesmo caminho
//! do app: `MotoresLocais` (download conferido por SHA-256 e carga), `Integracao::preparar`,
//! `Lancamento` (fila de arquivos, `ProcessadorDeArquivo`, coletor e processador da reunião) e a
//! decodificação da casca (Media Foundation para m4a no Windows).
//!
//! Só roda pedido: `DITASE_E2E=1 cargo test -p ditase --release e2e -- --ignored --nocapture`.
//! - `DITASE_MODELOS`: pasta dos modelos (Parakeet e falantes; os que faltarem são baixados e
//!   conferidos pelo baixador do app).
//! - `DITASE_E2E_LLAMA` e `DITASE_E2E_GGUF`: `llama-server` e um GGUF pequeno para o resumo.
//!   O app só aceita o Qwen do catálogo; aqui o resumo usa as mesmas funções de produção
//!   (`resumir_com`, `corpo_do_resumo`, `ler_resumo` e o canal TLS do `ServidorLlama`) com o GGUF
//!   pequeno. Sem as duas variáveis, a etapa do resumo é pulada (o CI sempre passa as duas).
//! - `DITASE_E2E_SAIDA`: pasta dos documentos em JSON (a exportação .txt/.srt/.md roda depois,
//!   com a função da interface) e de `tempos.json`.
//!
//! O runner não tem dispositivo de áudio: a reunião usa uma captura dublê que entrega PCM real do
//! corpus em tempo real, com o mesmo `RelogioDeFonte` da captura de verdade. Acelerar não serve:
//! pausa e encerramento são marcados pelo relógio de parede da reunião.
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

use motores::{
    llama::{self, ServidorLlama},
    medicao::erros_de_palavras,
};
use nucleo::motores::{IdentificadorDeFalantes, Limpador};
use serde_json::{json, Value};

use crate::{
    audio::{Fonte, TAXA},
    integracao::{Fala, FalaIndisponivel, Integracao, Motores},
    lancamento::{AoBloco, Avisar, Capturador, Lancamento, RelogioDeFonte},
    motores_locais::{self, MotoresLocais},
    preferencias::Preferencias,
};

/// Limites medidos (ver ENTREGA): folga sobre o observado no Mac e no runner.
const WER_ARQUIVO: f64 = 0.10;
const WER_REUNIAO: f64 = 0.15;
const WER_RECUPERADA: f64 = 0.15;
/// Palavras atribuídas à pessoa certa no arquivo de duas vozes. As duas vozes sintéticas
/// femininas confundem o CAM++ numa das seis falas (medido: ~80%; nenhum limiar resolve).
const ACERTO_FALANTES: f64 = 0.70;

// MARK: Corpus

fn raiz_do_repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
fn corpus(rel: &str) -> PathBuf {
    raiz_do_repo().join("motores/corpus").join(rel)
}
fn ler(rel: &str) -> Vec<f32> {
    motores::audio::ler_wav(&corpus(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}
fn texto(rel: &str) -> String {
    std::fs::read_to_string(corpus(rel))
        .unwrap()
        .trim()
        .to_string()
}
/// Conversa com vozes naturais (Luciana e Joana): base do WER. A de três vozes do corpus tem duas
/// vozes robóticas (Eloquence) que o Parakeet não reconhece; serve só para a separação de falantes.
const NATURAL: &str = "falantes/conversa-2-vozes";

fn segmentos(base: &str) -> Vec<(String, f64, f64, String)> {
    let v: Value = serde_json::from_str(&texto(&format!("{base}.json"))).unwrap();
    v.as_array()
        .unwrap()
        .iter()
        .map(|s| {
            (
                s["voz"].as_str().unwrap().to_string(),
                s["inicio"].as_f64().unwrap(),
                s["fim"].as_f64().unwrap(),
                s["texto"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}
fn amostra(s: f64) -> usize {
    (s * TAXA as f64).round() as usize
}
fn silencio(s: f64) -> Vec<f32> {
    vec![0.0; amostra(s)]
}
fn trecho(pcm: &[f32], de: f64, ate: f64) -> Vec<f32> {
    pcm[amostra(de).min(pcm.len())..amostra(ate).min(pcm.len())].to_vec()
}

// MARK: Registro de tempos e eventos

#[derive(Default)]
struct Tempos(Mutex<Vec<(String, f64)>>);
impl Tempos {
    fn anotar(&self, etapa: &str, s: f64) {
        println!("TEMPO {etapa}: {s:.2} s");
        self.0.lock().unwrap().push((etapa.to_string(), s));
    }
    fn json(&self) -> Value {
        Value::Object(
            self.0
                .lock()
                .unwrap()
                .iter()
                .map(|(k, v)| (k.clone(), json!((v * 100.0).round() / 100.0)))
                .collect(),
        )
    }
}

/// Primeiro instante de cada etapa de `ditado:progresso`, por documento.
#[derive(Default)]
struct Eventos(Mutex<Vec<(Instant, String, String)>>);
impl Eventos {
    fn avisar(self: &Arc<Self>) -> Avisar {
        let eu = self.clone();
        Arc::new(move |assunto, detalhe| {
            if assunto != "ditado:progresso" {
                return;
            }
            let Some(v) = detalhe else { return };
            let (Some(id), Some(etapa)) = (v["id"].as_str(), v["etapa"]["id"].as_str()) else {
                return;
            };
            let mut l = eu.0.lock().unwrap();
            if !l.iter().any(|(_, i, e)| i == id && e == etapa) {
                l.push((Instant::now(), id.to_string(), etapa.to_string()));
            }
        })
    }
    /// Etapas na ordem, com o instante de entrada. `id`: só desse documento (a reunião usa o id
    /// do documento; o arquivo usa o da versão, então ali vale o registro inteiro do serviço).
    fn etapas(&self, id: Option<&str>) -> Vec<(Instant, String)> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, i, _)| id.is_none_or(|x| x == i))
            .map(|(t, _, e)| (*t, e.clone()))
            .collect()
    }
}

fn esperar(prazo: Duration, o_que: &str, mut condicao: impl FnMut() -> bool) {
    let inicio = Instant::now();
    while !condicao() {
        assert!(inicio.elapsed() < prazo, "tempo esgotado esperando {o_que}");
        thread::sleep(Duration::from_millis(100));
    }
}

// MARK: Motores: os do app, com resumo por GGUF pequeno

/// Delega tudo ao `MotoresLocais` do app; só o resumo usa o GGUF pequeno.
struct MotoresDaProva {
    app: Arc<MotoresLocais>,
    resumo: Option<(Arc<ServidorLlama>, PathBuf)>,
}

impl Motores for MotoresDaProva {
    fn fala(&self, esperar: bool) -> Result<Arc<dyn Fala>, FalaIndisponivel> {
        self.app.fala(esperar)
    }
    fn fala_disponivel(&self) -> bool {
        self.app.fala_disponivel()
    }
    fn falantes(&self) -> Option<Arc<dyn IdentificadorDeFalantes>> {
        self.app.falantes()
    }
    fn limpador(&self) -> Option<Arc<dyn Limpador>> {
        None
    }
    fn resumo_disponivel(&self) -> bool {
        self.resumo.is_some()
    }
    fn resumir(
        &self,
        linhas: &[String],
        cancelado: &dyn Fn() -> bool,
    ) -> Result<Vec<String>, String> {
        let (servidor, gguf) = self.resumo.as_ref().ok_or("sem resumo")?;
        motores_locais::resumir_com(linhas, cancelado, &|dados| {
            let r = servidor
                .completar(
                    gguf,
                    &motores_locais::corpo_do_resumo(dados),
                    Duration::from_secs(120),
                )
                .map_err(|e| e.to_string())?;
            motores_locais::ler_resumo(&r)
        })
    }
    fn estado_reconhecedor(&self) -> (String, String) {
        self.app.estado_reconhecedor()
    }
    fn estado_limpador(&self) -> (String, String) {
        self.app.estado_limpador()
    }
    fn modelo_ia(&self) -> Value {
        self.app.modelo_ia()
    }
    fn baixar_modelo(&self) -> Result<(), String> {
        Err("fora da prova".into())
    }
    fn cancelar_modelo(&self) {}
    fn llama_instalado(&self) -> bool {
        self.resumo.is_some()
    }
    fn desligar(&self) {
        if let Some((s, _)) = &self.resumo {
            s.parar();
        }
    }
}

// MARK: Captura dublê com PCM real

/// Entrega o PCM de cada fonte em blocos de 100 ms no ritmo real, com o tempo pelo mesmo
/// `RelogioDeFonte` da captura de verdade. Depois do fim do PCM segue com silêncio até ser solta,
/// como um dispositivo aberto.
struct CapturaDeCorpus {
    sistema: Arc<Vec<f32>>,
    microfone: Arc<Vec<f32>>,
}

impl Capturador for CapturaDeCorpus {
    fn abrir(
        &self,
        fonte: Fonte,
        relogio: Instant,
        mut ao_bloco: AoBloco,
    ) -> Result<Box<dyn Send>, String> {
        let pcm = match fonte {
            Fonte::Sistema => self.sistema.clone(),
            Fonte::Microfone => self.microfone.clone(),
        };
        let fim = Arc::new(AtomicBool::new(false));
        let parar = fim.clone();
        let n = TAXA as usize / 10;
        let fio = thread::spawn(move || {
            let mut tempo = RelogioDeFonte::default();
            let inicio = Instant::now();
            let mut i = 0usize;
            while !parar.load(Ordering::Acquire) {
                let alvo = inicio + Duration::from_millis(100 * (i as u64 + 1));
                if let Some(falta) = alvo.checked_duration_since(Instant::now()) {
                    thread::sleep(falta);
                }
                let de = i * n;
                let bloco: Vec<f32> = (de..de + n)
                    .map(|k| pcm.get(k).copied().unwrap_or(0.0))
                    .collect();
                let nivel = bloco.iter().fold(0f32, |m, a| m.max(a.abs()));
                ao_bloco(
                    tempo.tempo(relogio.elapsed().as_secs_f64(), n),
                    bloco,
                    nivel,
                );
                i += 1;
            }
        });
        struct Guarda(Arc<AtomicBool>, Option<thread::JoinHandle<()>>);
        impl Drop for Guarda {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
                if let Some(f) = self.1.take() {
                    let _ = f.join();
                }
            }
        }
        Ok(Box::new(Guarda(fim, Some(fio))))
    }
}

// MARK: Conferências

fn texto_do_documento(doc: &Value) -> String {
    let mut trechos: Vec<&Value> = doc["trechos"].as_array().unwrap().iter().collect();
    trechos.sort_by(|a, b| {
        a["inicio"]
            .as_f64()
            .unwrap()
            .total_cmp(&b["inicio"].as_f64().unwrap())
    });
    trechos
        .iter()
        .map(|t| t["texto"].as_str().unwrap_or(""))
        .collect::<Vec<_>>()
        .join(" ")
}

fn vozes_usadas(doc: &Value) -> BTreeSet<String> {
    doc["trechos"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["voz"].as_str().unwrap_or("").to_string())
        .collect()
}

/// Fração das palavras com a pessoa certa: cada voz obtida vale a voz de referência em que mais
/// palavras dela caem (pelo meio da palavra estimado no trecho). "desconhecida" conta como erro.
fn acerto_de_falantes(doc: &Value, referencia: &[(String, f64, f64, String)]) -> f64 {
    let voz_em = |t: f64| {
        referencia
            .iter()
            .min_by(|a, b| {
                let d = |s: &(String, f64, f64, String)| {
                    if t < s.1 {
                        s.1 - t
                    } else if t > s.2 {
                        t - s.2
                    } else {
                        0.0
                    }
                };
                d(a).total_cmp(&d(b))
            })
            .map(|s| s.0.clone())
            .unwrap_or_default()
    };
    let mut pares: Vec<(String, String, usize)> = Vec::new();
    for t in doc["trechos"].as_array().unwrap() {
        let n = t["texto"].as_str().unwrap_or("").split_whitespace().count();
        let meio = (t["inicio"].as_f64().unwrap() + t["fim"].as_f64().unwrap()) / 2.0;
        pares.push((t["voz"].as_str().unwrap_or("").to_string(), voz_em(meio), n));
    }
    let total: usize = pares.iter().map(|p| p.2).sum();
    let mut contagem: std::collections::HashMap<(String, String), usize> = Default::default();
    for (h, r, n) in &pares {
        *contagem.entry((h.clone(), r.clone())).or_default() += n;
    }
    let mut certas = 0;
    for h in pares.iter().map(|p| p.0.clone()).collect::<BTreeSet<_>>() {
        if h == "desconhecida" {
            continue;
        }
        certas += contagem
            .iter()
            .filter(|((hh, _), _)| *hh == h)
            .map(|(_, n)| *n)
            .max()
            .unwrap_or(0);
    }
    certas as f64 / total.max(1) as f64
}

fn wer(referencia: &str, obtido: &str) -> f64 {
    let (e, n) = erros_de_palavras(referencia, obtido);
    e as f64 / n.max(1) as f64
}

fn salvar(saida: &Option<PathBuf>, nome: &str, v: &Value) {
    if let Some(d) = saida {
        std::fs::create_dir_all(d).unwrap();
        std::fs::write(d.join(nome), serde_json::to_vec_pretty(v).unwrap()).unwrap();
    }
}

/// Duração de cada etapa de `ditado:progresso` até a seguinte (a última, até `fim`).
fn anotar_etapas(tempos: &Tempos, eventos: &Eventos, prefixo: &str, fim: Instant) {
    let etapas = eventos.etapas(None);
    for (k, (t, nome)) in etapas.iter().enumerate() {
        let ate = etapas.get(k + 1).map(|(t2, _)| *t2).unwrap_or(fim);
        tempos.anotar(
            &format!("{prefixo}.{nome}"),
            ate.duration_since(*t).as_secs_f64(),
        );
    }
}

// MARK: A prova

#[test]
#[ignore = "modelos reais (~700 MB) e cerca de 3 min; rodar com DITASE_E2E=1 e --ignored"]
fn e2e_arquivo_reuniao_resumo_e_recuperacao() {
    if std::env::var("DITASE_E2E").as_deref() != Ok("1") {
        eprintln!("DITASE_E2E=1 não definido: prova de ponta a ponta pulada");
        return;
    }
    let tmp = crate::caminhos::pasta_de_teste();
    // Pasta do app (registro, trabalho do llama) isolada nesta execução.
    if std::env::var_os("DITASE_RAIZ").is_none() {
        std::env::set_var("DITASE_RAIZ", tmp.path().join("App"));
    }
    let saida = std::env::var_os("DITASE_E2E_SAIDA").map(PathBuf::from);
    let tempos = Arc::new(Tempos::default());
    let total = Instant::now();

    // 1. Modelos pelo caminho do app: confere (ou baixa) por SHA-256 e carrega.
    let pasta_modelos = motores::pasta_de_teste();
    let prefs = Arc::new(Preferencias::abrir(tmp.path().join("preferencias.json")));
    let app = MotoresLocais::novo(pasta_modelos.clone(), prefs, Arc::new(|| {}));
    let t = Instant::now();
    app.preparar_fala();
    app.fala(true)
        .unwrap_or_else(|e| panic!("reconhecedor indisponível: {e}"));
    tempos.anotar(
        "modelos.parakeet_conferir_e_carregar",
        t.elapsed().as_secs_f64(),
    );
    let t = Instant::now();
    esperar(Duration::from_secs(600), "modelo de falantes", || {
        app.falantes().is_some()
    });
    tempos.anotar(
        "modelos.falantes_conferir_e_carregar",
        t.elapsed().as_secs_f64(),
    );

    let resumo = match (
        std::env::var_os("DITASE_E2E_LLAMA"),
        std::env::var_os("DITASE_E2E_GGUF"),
    ) {
        (Some(bin), Some(gguf)) => {
            let ajustes = llama::Ajustes {
                prazo_de_subida: Duration::from_secs(120),
                pasta_de_trabalho: Some(tmp.path().join("App").join("Trabalho").join("llama")),
                ..Default::default()
            };
            let servidor = ServidorLlama::novo(PathBuf::from(bin), llama::PROMPT_LIMPEZA, ajustes);
            Some((servidor, PathBuf::from(gguf)))
        }
        _ => {
            println!("AVISO: sem DITASE_E2E_LLAMA/DITASE_E2E_GGUF; resumo pulado");
            None
        }
    };
    let com_resumo = resumo.is_some();
    let motores: Arc<dyn Motores> = Arc::new(MotoresDaProva { app, resumo });

    let mut relatorio = json!({});
    arquivo(&tmp, &motores, &tempos, &saida, &mut relatorio);
    reuniao(&tmp, &motores, &tempos, &saida, com_resumo, &mut relatorio);
    recuperacao(&tmp, &motores, &tempos, &saida, &mut relatorio);
    motores.desligar();

    tempos.anotar("total", total.elapsed().as_secs_f64());
    relatorio["tempos"] = tempos.json();
    relatorio["plataforma"] = json!(format!(
        "{}-{}",
        std::env::consts::OS,
        std::env::consts::ARCH
    ));
    relatorio["threads_cpu"] = json!(thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(0));
    println!("RELATORIO {}", serde_json::to_string(&relatorio).unwrap());
    salvar(&saida, "relatorio.json", &relatorio);
}

fn servico(
    pasta: &Path,
    motores: &Arc<dyn Motores>,
    captura: Arc<dyn Capturador>,
    eventos: &Arc<Eventos>,
) -> Arc<Lancamento> {
    std::fs::create_dir_all(pasta).unwrap();
    let banco = pasta.join("ditado.sqlite");
    // Mesmo `preparar` do app: regras de fala e dicionário do banco.
    let integracao = Arc::new(Integracao::abrir(&banco, motores.clone()).unwrap());
    let preparar: crate::lancamento::Preparar = Arc::new(move |t| integracao.preparar(t));
    Lancamento::new(
        &banco,
        &pasta.join("ReunioesAudio"),
        motores.clone(),
        captura,
        preparar,
        eventos.avisar(),
    )
    .unwrap()
}

fn captura_vazia() -> Arc<dyn Capturador> {
    Arc::new(CapturaDeCorpus {
        sistema: Arc::new(Vec::new()),
        microfone: Arc::new(Vec::new()),
    })
}

// MARK: 1. Arquivo

fn arquivo(
    tmp: &tempfile::TempDir,
    motores: &Arc<dyn Motores>,
    tempos: &Tempos,
    saida: &Option<PathBuf>,
    relatorio: &mut Value,
) {
    let referencia_de = |base: &str| {
        segmentos(base)
            .iter()
            .map(|s| s.3.clone())
            .collect::<Vec<_>>()
            .join(" ")
    };

    // O m4a tem de passar pelo Media Foundation no Windows (sem cair no Symphonia).
    #[cfg(windows)]
    {
        let m4a = corpus(&format!("{NATURAL}.m4a"));
        let duracao = ler(&format!("{NATURAL}.wav")).len() as f64 / TAXA as f64;
        let t = Instant::now();
        let pcm = crate::nativo::midia::decodificar(&m4a)
            .unwrap_or_else(|e| panic!("Media Foundation não decodificou o m4a: {e}"));
        tempos.anotar("arquivo.m4a.media_foundation", t.elapsed().as_secs_f64());
        let s = pcm.len() as f64 / TAXA as f64;
        assert!(
            (s - duracao).abs() < 0.5,
            "Media Foundation devolveu {s:.2} s; esperado {duracao:.2} s"
        );
    }

    // A separação de três vozes fica no teste `diarizacao_separa_tres_vozes` dos motores (o
    // Parakeet não transcreve as vozes robóticas, então os trechos não mostrariam as três).
    for (rotulo, base, ext, falantes) in [("wav", NATURAL, "wav", 2), ("m4a", NATURAL, "m4a", 2)] {
        let caminho = corpus(&format!("{base}.{ext}"));
        let referencia = referencia_de(base);
        let duracao = ler(&format!("{base}.wav")).len() as f64 / TAXA as f64;
        let eventos = Arc::new(Eventos::default());
        let s = servico(
            &tmp.path().join(format!("Arquivo-{rotulo}")),
            motores,
            captura_vazia(),
            &eventos,
        );
        let t = Instant::now();
        let id = s.adicionar(&[caminho]).unwrap().unwrap();
        // A fase viva vem da lista de `estado` (a fila sobrepõe a do banco), como na interface.
        let fase = |s: &Lancamento| {
            s.estado(true).unwrap()["arquivos"]
                .as_array()
                .unwrap()
                .iter()
                .find(|a| a["id"] == json!(id))
                .map(|a| a["fase"].as_str().unwrap_or("").to_string())
                .unwrap_or_default()
        };
        esperar(Duration::from_secs(300), "arquivo pronto", || {
            ["pronto", "falhou", "semFala", "cancelado"].contains(&fase(&s).as_str())
        });
        let fim = Instant::now();
        let processamento = t.elapsed().as_secs_f64();
        let doc = s.documento(&id).unwrap();
        salvar(saida, &format!("arquivo-{rotulo}.json"), &doc);
        assert_eq!(doc["fase"], "pronto", "arquivo {rotulo}: {doc}");
        // A versão publicada pelo processador é a que o documento mostra.
        let estado = s.estado(true).unwrap();
        let item = estado["arquivos"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["id"] == json!(id))
            .cloned()
            .unwrap();
        assert_eq!(item["fase"], "pronto", "lista de arquivos: {item}");
        tempos.anotar(&format!("arquivo.{rotulo}.total"), processamento);
        tempos.anotar(&format!("arquivo.{rotulo}.rtf"), processamento / duracao);
        anotar_etapas(tempos, &eventos, &format!("arquivo.{rotulo}"), fim);

        let obtido = texto_do_documento(&doc);
        let w = wer(&referencia, &obtido);
        let vozes = vozes_usadas(&doc);
        println!(
            "arquivo {rotulo}: WER {:.2}%, vozes {vozes:?}\n  {obtido}",
            w * 100.0
        );
        assert!(
            w <= WER_ARQUIVO,
            "arquivo {rotulo}: WER {:.2}% acima de {:.0}%: {obtido}",
            w * 100.0,
            WER_ARQUIVO * 100.0
        );
        let pessoas = vozes.iter().filter(|v| *v != "desconhecida").count();
        assert_eq!(pessoas, falantes, "arquivo {rotulo}: falantes {vozes:?}");
        let acerto = acerto_de_falantes(&doc, &segmentos(base));
        println!(
            "arquivo {rotulo}: palavras com a pessoa certa {:.1}%",
            acerto * 100.0
        );
        assert!(
            acerto >= ACERTO_FALANTES,
            "arquivo {rotulo}: só {:.1}% das palavras com a pessoa certa",
            acerto * 100.0
        );
        relatorio[format!("arquivo_{rotulo}")] = json!({
            "wer_pct": (w * 10_000.0).round() / 100.0, "falantes": pessoas,
            "acerto_falantes_pct": (acerto * 1000.0).round() / 10.0,
            "trechos": doc["trechos"].as_array().unwrap().len(),
        });
        s.desligar(Duration::from_secs(2));
        assert!(s.esperar_ocioso(Duration::from_secs(10)));
    }
}

// MARK: 2. Reunião com pausa, microfone e resumo

/// Faixas da reunião. Sistema: conversa natural (2 vozes) com uma frase na janela pausada (não pode
/// aparecer no texto) e um silêncio em que o microfone fala. Microfone: silêncio e uma frase.
struct Roteiro {
    sistema: Vec<f32>,
    microfone: Vec<f32>,
    referencia: String,
    proibidas: Vec<&'static str>,
    pausar_em: f64,
    retomar_em: f64,
    fim: f64,
}

fn roteiro() -> Roteiro {
    let conversa = ler(&format!("{NATURAL}.wav"));
    let seg = segmentos(NATURAL);
    // Cortes no meio dos silêncios entre falas.
    let corte1 = (seg[2].2 + seg[3].1) / 2.0;
    let corte2 = (seg[3].2 + seg[4].1) / 2.0;
    let na_pausa = ler("luciana-04.wav");
    let do_microfone = ler("luciana-02.wav");
    let s = |v: &[f32]| v.len() as f64 / TAXA as f64;

    let mut sistema = trecho(&conversa, 0.0, corte1);
    let pausa_ini = s(&sistema);
    sistema.extend(silencio(2.0));
    sistema.extend(&na_pausa);
    sistema.extend(silencio(2.0));
    let pausa_fim = s(&sistema);
    sistema.extend(trecho(&conversa, corte1, corte2));
    let mic_ini = s(&sistema) + 1.0;
    sistema.extend(silencio(1.0 + s(&do_microfone) + 1.0));
    sistema.extend(trecho(&conversa, corte2, s(&conversa)));
    let fim = s(&sistema);

    let mut microfone = silencio(mic_ini);
    microfone.extend(&do_microfone);
    microfone.resize(sistema.len(), 0.0);

    let mut referencia: Vec<String> = seg[..4].iter().map(|x| x.3.clone()).collect();
    referencia.push(texto("luciana-02.txt"));
    referencia.extend(seg[4..].iter().map(|x| x.3.clone()));
    Roteiro {
        sistema,
        microfone,
        referencia: referencia.join(" "),
        proibidas: vec!["audiência", "conciliação", "remarcada"],
        pausar_em: pausa_ini + 1.0,
        retomar_em: pausa_fim - 1.0,
        fim,
    }
}

fn reuniao(
    tmp: &tempfile::TempDir,
    motores: &Arc<dyn Motores>,
    tempos: &Tempos,
    saida: &Option<PathBuf>,
    com_resumo: bool,
    relatorio: &mut Value,
) {
    let r = roteiro();
    let eventos = Arc::new(Eventos::default());
    let s = servico(
        &tmp.path().join("Reuniao"),
        motores,
        Arc::new(CapturaDeCorpus {
            sistema: Arc::new(r.sistema.clone()),
            microfone: Arc::new(r.microfone.clone()),
        }),
        &eventos,
    );
    let id = s.iniciar_reuniao("sistemaEMicrofone").unwrap();
    let inicio = Instant::now();
    let ate = |t: f64| {
        if let Some(falta) =
            (inicio + Duration::from_secs_f64(t)).checked_duration_since(Instant::now())
        {
            thread::sleep(falta);
        }
    };
    ate(r.pausar_em);
    s.pausar(&id).unwrap();
    assert_eq!(s.documento(&id).unwrap()["fase"], "pausada");
    ate(r.retomar_em);
    s.pausar(&id).unwrap();
    assert_eq!(s.documento(&id).unwrap()["fase"], "gravando");
    ate(r.fim + 1.0);
    // Trechos ao vivo já devem ter saído durante a gravação.
    let ao_vivo = s.documento(&id).unwrap()["trechos"]
        .as_array()
        .unwrap()
        .len();
    let encerrou = Instant::now();
    s.encerrar(&id).unwrap();
    esperar(Duration::from_secs(300), "reunião pronta", || {
        let d = s.documento(&id).unwrap();
        (d["fase"] == "pronta" || d["fase"] == "interrompida")
            && s.estado(true).unwrap().get("reuniao").is_none()
    });
    let fim = Instant::now();
    tempos.anotar(
        "reuniao.gravacao_tempo_real",
        encerrou.duration_since(inicio).as_secs_f64(),
    );
    tempos.anotar(
        "reuniao.do_encerrar_ao_pronto",
        fim.duration_since(encerrou).as_secs_f64(),
    );
    let finais: Vec<(Instant, String)> = eventos
        .etapas(Some(&id))
        .into_iter()
        .filter(|(t, e)| *t >= encerrou && e != "gravando")
        .collect();
    for (k, (t, nome)) in finais.iter().enumerate() {
        let ate = finais.get(k + 1).map(|(t2, _)| *t2).unwrap_or(fim);
        tempos.anotar(
            &format!("reuniao.{nome}"),
            ate.duration_since(*t).as_secs_f64(),
        );
    }

    let doc = s.documento(&id).unwrap();
    salvar(saida, "reuniao.json", &doc);
    assert_eq!(doc["fase"], "pronta", "{doc}");
    assert_eq!(doc["falantes"], "identificados", "{doc}");
    let obtido = texto_do_documento(&doc);
    let w = wer(&r.referencia, &obtido);
    let vozes = vozes_usadas(&doc);
    println!(
        "reunião: WER {:.2}%, vozes {vozes:?}, trechos ao vivo antes de encerrar {ao_vivo}\n  {obtido}",
        w * 100.0
    );
    assert!(ao_vivo > 0, "nenhum trecho ao vivo durante a gravação");
    assert!(
        w <= WER_REUNIAO,
        "reunião: WER {:.2}% acima de {:.0}%: {obtido}",
        w * 100.0,
        WER_REUNIAO * 100.0
    );
    let normal = obtido.to_lowercase();
    for p in &r.proibidas {
        assert!(
            !normal.contains(p),
            "fala da pausa entrou no texto ({p}): {obtido}"
        );
    }
    assert!(
        vozes.contains("voce"),
        "microfone sem a voz \"voce\": {vozes:?}"
    );
    let do_sistema = vozes
        .iter()
        .filter(|v| *v != "voce" && *v != "desconhecida")
        .count();
    assert_eq!(do_sistema, 2, "vozes do sistema: {vozes:?}");
    // Sem lacunas falsas: o áudio chegou contínuo, inclusive na pausa.
    let lacunas = doc
        .get("lacunas")
        .and_then(Value::as_array)
        .map_or(0, |l| l.len());
    let pendentes = doc
        .get("pendentes")
        .and_then(Value::as_array)
        .map_or(0, |l| l.len());
    assert_eq!(lacunas, 0, "lacunas falsas: {}", doc["lacunas"]);
    assert_eq!(pendentes, 0, "trechos pendentes: {}", doc["pendentes"]);
    // Duração sem o tempo pausado.
    let duracao = doc["duracao"].as_f64().unwrap_or(0.0);
    let esperada = r.fim + 1.0 - (r.retomar_em - r.pausar_em);
    assert!(
        (duracao - esperada).abs() < 2.0,
        "duração {duracao:.1} s; esperada {esperada:.1} s"
    );

    let mut resumo = Value::Null;
    if com_resumo {
        let topicos = doc["resumo"].as_array().cloned().unwrap_or_default();
        println!("resumo: {topicos:?} erro: {}", doc["erroResumo"]);
        assert!(
            !topicos.is_empty(),
            "resumo vazio; erroResumo: {}",
            doc["erroResumo"]
        );
        resumo = json!(topicos);
    }
    relatorio["reuniao"] = json!({
        "wer_pct": (w * 10_000.0).round() / 100.0, "vozes": vozes, "lacunas": lacunas,
        "trechos_ao_vivo": ao_vivo, "duracao_s": duracao, "resumo": resumo,
    });
    s.desligar(Duration::from_secs(2));
    assert!(s.esperar_ocioso(Duration::from_secs(10)));
}

// MARK: 3. Reunião interrompida e recuperada

fn recuperacao(
    tmp: &tempfile::TempDir,
    motores: &Arc<dyn Motores>,
    tempos: &Tempos,
    saida: &Option<PathBuf>,
    relatorio: &mut Value,
) {
    let seg = segmentos(NATURAL);
    let pasta = tmp.path().join("Recuperada");
    // Fecha o app no silêncio depois da segunda fala.
    let fechar_em = (seg[1].2 + seg[2].1) / 2.0;
    let referencia = format!("{} {}", seg[0].3, seg[1].3);
    let eventos = Arc::new(Eventos::default());
    let id = {
        let s = servico(
            &pasta,
            motores,
            Arc::new(CapturaDeCorpus {
                sistema: Arc::new(ler(&format!("{NATURAL}.wav"))),
                microfone: Arc::new(Vec::new()),
            }),
            &eventos,
        );
        let id = s.iniciar_reuniao("sistema").unwrap();
        thread::sleep(Duration::from_secs_f64(fechar_em));
        // O app fechando no meio: só o coletor grava o que estava em memória.
        let t = Instant::now();
        s.desligar(Duration::from_secs(2));
        tempos.anotar("recuperacao.desligar", t.elapsed().as_secs_f64());
        assert!(s.esperar_ocioso(Duration::from_secs(30)));
        id
    };
    // Reabre como na próxima abertura do app.
    let t = Instant::now();
    let s = servico(&pasta, motores, captura_vazia(), &eventos);
    tempos.anotar("recuperacao.reabrir_servico", t.elapsed().as_secs_f64());
    let recuperaveis = s.estado(true).unwrap()["recuperaveis"].clone();
    assert!(
        recuperaveis
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["id"] == json!(id)),
        "reunião interrompida não ficou recuperável: {recuperaveis}"
    );
    let t = Instant::now();
    s.recuperar_reuniao(&id).unwrap();
    esperar(Duration::from_secs(300), "reunião recuperada", || {
        let d = s.documento(&id).unwrap();
        (d["fase"] == "pronta" || d["fase"] == "interrompida")
            && s.estado(true).unwrap().get("reuniao").is_none()
    });
    tempos.anotar("recuperacao.processar", t.elapsed().as_secs_f64());
    let doc = s.documento(&id).unwrap();
    salvar(saida, "recuperada.json", &doc);
    assert_eq!(doc["fase"], "pronta", "{doc}");
    let obtido = texto_do_documento(&doc);
    let w = wer(&referencia, &obtido);
    println!("recuperada: WER {:.2}%\n  {obtido}", w * 100.0);
    assert!(
        w <= WER_RECUPERADA,
        "recuperada: WER {:.2}% acima de {:.0}%: {obtido}",
        w * 100.0,
        WER_RECUPERADA * 100.0
    );
    let lacunas = doc
        .get("lacunas")
        .and_then(Value::as_array)
        .map_or(0, |l| l.len());
    assert_eq!(
        lacunas, 0,
        "lacunas falsas na recuperada: {}",
        doc["lacunas"]
    );
    relatorio["recuperada"] = json!({
        "wer_pct": (w * 10_000.0).round() / 100.0, "vozes": vozes_usadas(&doc), "lacunas": lacunas,
    });
    s.desligar(Duration::from_secs(2));
    assert!(s.esperar_ocioso(Duration::from_secs(30)));
}
