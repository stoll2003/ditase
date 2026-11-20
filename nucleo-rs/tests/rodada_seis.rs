mod comum;

use nucleo::{motores::*, *};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, AtomicUsize, Ordering},
};

const TAXA_USIZE: usize = TAXA as usize;

fn novo_uuid() -> String {
    static N: AtomicU64 = AtomicU64::new(1000);
    let n = N.fetch_add(1, Ordering::Relaxed);
    format!("{n:08x}-0000-4000-8000-000000000000")
}

struct Oraculo {
    roteiro: Vec<PalavraReconhecida>,
    chamadas: AtomicUsize,
}

impl Oraculo {
    fn new(roteiro: Vec<PalavraReconhecida>) -> Self {
        Self {
            roteiro,
            chamadas: AtomicUsize::new(0),
        }
    }
    fn chamadas(&self) -> usize {
        self.chamadas.load(Ordering::Relaxed)
    }
}

impl ReconhecedorDeFala for Oraculo {
    fn palavras(
        &self,
        amostras: &[f32],
        inicio: f64,
        fim_da_fala: Option<f64>,
    ) -> Result<Vec<PalavraReconhecida>, Erro> {
        self.chamadas.fetch_add(1, Ordering::Relaxed);
        let fim = inicio + fim_da_fala.unwrap_or(amostras.len() as f64 / f64::from(TAXA));
        Ok(self
            .roteiro
            .iter()
            .filter_map(|p| {
                if p.fim <= inicio || p.inicio >= fim {
                    return None;
                }
                if p.inicio >= inicio - 1e-9 && p.fim <= fim + 1e-9 {
                    return Some(p.clone());
                }
                let pref: String = p.texto.chars().take(3).collect();
                Some(PalavraReconhecida {
                    texto: format!("{pref}~"),
                    inicio: p.inicio.max(inicio),
                    fim: p.fim.min(fim),
                })
            })
            .collect())
    }
}

struct Identificador(Vec<FaixaDeVoz>);
impl IdentificadorDeFalantes for Identificador {
    fn faixas(&self, wav: &Path) -> Result<Vec<FaixaDeVoz>, Erro> {
        assert!(wav.exists());
        Ok(self.0.clone())
    }
}

fn pcm_sintetico(rot: &[PalavraReconhecida], segundos: f64) -> Vec<f32> {
    let mut saida = vec![0f32; (segundos * f64::from(TAXA)) as usize];
    for p in rot {
        let a = (p.inicio * f64::from(TAXA)) as usize;
        let z = saida.len().min((p.fim * f64::from(TAXA)) as usize);
        for (i, s) in saida.iter_mut().enumerate().take(z).skip(a) {
            *s = 0.1 * ((i as f64) * 0.3).sin() as f32;
        }
    }
    saida
}

struct Ambiente {
    tmp: tempfile::TempDir,
    banco: PathBuf,
    armazem: ArmazemLancamento,
    disco: DiscoDeBlocos,
}

impl Ambiente {
    fn new() -> Self {
        let tmp = comum::tempdir();
        let banco = tmp.path().join("teste.sqlite");
        let armazem = ArmazemLancamento::new(&banco).unwrap();
        let disco = DiscoDeBlocos::new(tmp.path().join("Audio"));
        Self {
            tmp,
            banco,
            armazem,
            disco,
        }
    }
    fn reabrir(&self) -> ArmazemLancamento {
        ArmazemLancamento::new(&self.banco).unwrap()
    }
    fn nova_reuniao(&self) -> String {
        let id = novo_uuid();
        self.armazem
            .criar_documento(&id, "reuniao", "Reunião sintética", Some("mac"), "gravando")
            .unwrap();
        id
    }
    fn capturar(&self, coletor: &mut ColetorDeReuniao, pcm: &[f32]) {
        let passo = TAXA_USIZE / 10;
        let mut i = 0;
        while i < pcm.len() {
            let fim = pcm.len().min(i + passo);
            coletor
                .receber(FonteDeAudio::Mac, i as f64 / f64::from(TAXA), &pcm[i..fim])
                .unwrap();
            i += passo;
        }
    }
}

fn palavras(armazem: &ArmazemLancamento, doc: &str) -> Vec<String> {
    let mut trechos = armazem
        .documento(doc)
        .unwrap()
        .map(|d| d.trechos)
        .unwrap_or_default();
    trechos.sort_by(|a, b| a.inicio.total_cmp(&b.inicio));
    trechos
        .into_iter()
        .flat_map(|t| {
            t.texto
                .split_whitespace()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .collect()
}

fn inserir_trecho_raw(
    c: &rusqlite::Connection,
    doc: &str,
    id: &str,
    inicio: f64,
    fim: f64,
    texto: &str,
) {
    c.execute(
        "INSERT INTO lt_trechos(id,documento,versao,fonte,inicio,fim,texto,voz,criado_em) VALUES(?,?,'','mac',?,?,?,'chamada',0)",
        rusqlite::params![id, doc, inicio, fim, texto],
    )
    .unwrap();
}

#[test]
fn orfao_anterior_ao_ponto_de_costura_existente_vira_pendencia_e_transcreve() {
    let amb = Ambiente::new();
    let doc = amb.nova_reuniao();
    let rot = vec![
        PalavraReconhecida {
            texto: "primeira".into(),
            inicio: 0.2,
            fim: 0.6,
        },
        PalavraReconhecida {
            texto: "terceira".into(),
            inicio: 2.2,
            fim: 2.6,
        },
    ];
    let pcm = pcm_sintetico(&rot, 3.);
    let c = rusqlite::Connection::open(&amb.banco).unwrap();
    c.execute("INSERT INTO lt_costuras (documento, fonte, geracao, confirmado_ate) VALUES (?, 'mac', 0, 32000)", [&doc]).unwrap();
    for (seq, inicio) in [(0u64, 0usize), (2, 32000)] {
        amb.disco
            .gravar(
                IdBloco {
                    documento: doc.clone(),
                    fonte: FonteDeAudio::Mac,
                    geracao: 0,
                    seq,
                },
                inicio,
                &pcm[inicio..inicio + TAXA_USIZE],
            )
            .unwrap();
    }
    let mut reaberto = amb.reabrir();
    assert_eq!(reaberto.preparar_abertura(&amb.disco).adotados, 2);
    assert_eq!(reaberto.costuras(&doc).unwrap()[0].confirmado_ate, 32000);
    let abertas: Vec<usize> = reaberto
        .pendencias(&doc)
        .unwrap()
        .into_iter()
        .filter(|p| p.estado == "aberta")
        .map(|p| p.inicio)
        .collect();
    assert_eq!(abertas, vec![0]);
    let oraculo = Oraculo::new(rot);
    let id = Identificador(vec![FaixaDeVoz {
        voz: "pessoa-1".into(),
        inicio: 0.,
        fim: 3.,
    }]);
    {
        let mut p = ProcessadorDeReuniao::new(&doc, &mut reaberto, &amb.disco, &oraculo);
        p.recuperar().unwrap();
        p.encerrar_costuras().unwrap();
        assert!(p.passada_final(Some(&id)).unwrap());
    }
    assert_eq!(palavras(&reaberto, &doc), vec!["primeira", "terceira"]);
}

fn adotado_sobre_texto(
    gravado: (f64, f64, &str),
    rot: Vec<PalavraReconhecida>,
    inicio_bloco: usize,
) {
    let amb = Ambiente::new();
    let doc = amb.nova_reuniao();
    let c = rusqlite::Connection::open(&amb.banco).unwrap();
    c.execute("INSERT INTO lt_costuras (documento, fonte, geracao, confirmado_ate, fechada) VALUES (?, 'mac', 0, 16000, 1)", [&doc])
        .unwrap();
    let amostra = (gravado.0 * f64::from(TAXA)).round() as usize;
    inserir_trecho_raw(
        &c,
        &doc,
        &format!("{doc}|mac|0|{amostra}"),
        gravado.0,
        gravado.1,
        gravado.2,
    );
    let pcm = pcm_sintetico(&rot, 1.6);
    let bloco = amb
        .disco
        .gravar(
            IdBloco {
                documento: doc.clone(),
                fonte: FonteDeAudio::Mac,
                geracao: 0,
                seq: 0,
            },
            inicio_bloco,
            &pcm[inicio_bloco..inicio_bloco + TAXA_USIZE],
        )
        .unwrap();
    let mut reaberto = amb.reabrir();
    assert_eq!(reaberto.preparar_abertura(&amb.disco).adotados, 1);
    let oraculo = Oraculo::new(rot);
    let id = Identificador(vec![FaixaDeVoz {
        voz: "pessoa-1".into(),
        inicio: 0.,
        fim: 1.,
    }]);
    {
        let mut p = ProcessadorDeReuniao::new(&doc, &mut reaberto, &amb.disco, &oraculo);
        p.recuperar().unwrap();
        p.encerrar_costuras().unwrap();
        assert!(
            p.passada_final(Some(&id)).is_err(),
            "passada final não pode liberar áudio com pendência aberta"
        );
    }
    let textos: Vec<String> = reaberto
        .documento(&doc)
        .unwrap()
        .unwrap()
        .trechos
        .into_iter()
        .map(|t| t.texto)
        .collect();
    assert_eq!(textos, vec![gravado.2]);
    assert!(amb.disco.existe(&bloco));
    let abertas: Vec<_> = reaberto
        .pendencias(&doc)
        .unwrap()
        .into_iter()
        .filter(|p| p.estado == "aberta")
        .collect();
    assert_eq!(abertas.len(), 1);
    assert!(
        abertas[0].motivo.contains("já tem texto"),
        "{}",
        abertas[0].motivo
    );
}

#[test]
fn adotado_sobre_texto_igual_fica_retido_sem_duplicar() {
    adotado_sobre_texto(
        (0.2, 0.6, "primeira"),
        vec![PalavraReconhecida {
            texto: "primeira".into(),
            inicio: 0.2,
            fim: 0.6,
        }],
        0,
    );
}

#[test]
fn adotado_sobre_texto_com_palavra_faltando_fica_retido() {
    adotado_sobre_texto(
        (0.10, 0.80, "ligue amanhã"),
        vec![
            PalavraReconhecida {
                texto: "ligue".into(),
                inicio: 0.10,
                fim: 0.25,
            },
            PalavraReconhecida {
                texto: "nunca".into(),
                inicio: 0.35,
                fim: 0.50,
            },
            PalavraReconhecida {
                texto: "amanhã".into(),
                inicio: 0.60,
                fim: 0.80,
            },
        ],
        0,
    );
}

#[test]
fn adotado_que_comeca_dentro_de_trecho_gravado_fica_retido() {
    adotado_sobre_texto(
        (0.10, 0.88, "ligue amanhã"),
        vec![
            PalavraReconhecida {
                texto: "amanhã".into(),
                inicio: 0.55,
                fim: 0.88,
            },
            PalavraReconhecida {
                texto: "cedo".into(),
                inicio: 0.90,
                fim: 0.98,
            },
        ],
        9600,
    );
}

#[test]
fn adotado_sobre_texto_com_repeticao_ou_acento_fica_retido() {
    adotado_sobre_texto(
        (0.05, 0.90, "não"),
        vec![
            PalavraReconhecida {
                texto: "não".into(),
                inicio: 0.10,
                fim: 0.20,
            },
            PalavraReconhecida {
                texto: "não".into(),
                inicio: 0.60,
                fim: 0.85,
            },
        ],
        0,
    );
    adotado_sobre_texto(
        (0.05, 0.90, "avô"),
        vec![
            PalavraReconhecida {
                texto: "avó".into(),
                inicio: 0.10,
                fim: 0.30,
            },
            PalavraReconhecida {
                texto: "avô".into(),
                inicio: 0.50,
                fim: 0.80,
            },
        ],
        0,
    );
}

#[test]
fn pendencia_normal_recupera_segunda_ocorrencia() {
    let amb = Ambiente::new();
    let doc = amb.nova_reuniao();
    let rot = vec![
        PalavraReconhecida {
            texto: "não".into(),
            inicio: 0.10,
            fim: 0.20,
        },
        PalavraReconhecida {
            texto: "não".into(),
            inicio: 0.60,
            fim: 0.85,
        },
    ];
    let mut coletor = ColetorDeReuniao::new(doc.clone(), amb.reabrir(), amb.disco.clone(), 0.);
    coletor.abrir(FonteDeAudio::Mac, 0.).unwrap();
    amb.capturar(&mut coletor, &pcm_sintetico(&rot, 1.));
    coletor.encerrar(1.).unwrap();
    let c = rusqlite::Connection::open(&amb.banco).unwrap();
    c.execute(
        "INSERT INTO lt_costuras (documento, fonte, geracao, confirmado_ate, fechada) VALUES (?, 'mac', 0, 16000, 1) \
         ON CONFLICT DO UPDATE SET confirmado_ate = 16000, fechada = 1",
        [&doc],
    )
    .unwrap();
    inserir_trecho_raw(&c, &doc, &format!("{doc}|mac|0|1600"), 0.10, 0.20, "não");
    c.execute(
        "INSERT INTO lt_pendencias (id, documento, fonte, geracao, inicio, fim, motivo, estado) VALUES (?, ?, 'mac', 0, 6400, 16000, 'voz sem texto', 'aberta')",
        rusqlite::params![format!("{doc}|mac|0|6400"), doc],
    )
    .unwrap();
    let outro = amb.nova_reuniao();
    let id_em_uso = format!("{doc}|mac|0|9600");
    inserir_trecho_raw(&c, &outro, &id_em_uso, 0.60, 0.85, "síntese");
    let oraculo = Oraculo::new(rot);
    let mut arm = amb.reabrir();
    let novos = ProcessadorDeReuniao::new(&doc, &mut arm, &amb.disco, &oraculo)
        .repetir_pendencias()
        .unwrap();
    assert_eq!(palavras(&amb.armazem, &doc), vec!["não", "não"]);
    assert_eq!(novos.len(), 1);
    assert_eq!(novos[0].id, format!("{id_em_uso}#1"));
    assert!(amb
        .armazem
        .trechos(&doc)
        .unwrap()
        .iter()
        .any(|t| t.id == novos[0].id));
}

#[test]
fn pendencia_sobre_texto_retida_sem_reconhecer_e_a_seguinte_resolve() {
    let amb = Ambiente::new();
    let doc = amb.nova_reuniao();
    let rot = vec![
        PalavraReconhecida {
            texto: "ligue".into(),
            inicio: 0.10,
            fim: 0.25,
        },
        PalavraReconhecida {
            texto: "nunca".into(),
            inicio: 0.35,
            fim: 0.50,
        },
        PalavraReconhecida {
            texto: "amanhã".into(),
            inicio: 0.60,
            fim: 0.80,
        },
        PalavraReconhecida {
            texto: "depois".into(),
            inicio: 2.20,
            fim: 2.60,
        },
    ];
    let mut coletor = ColetorDeReuniao::new(doc.clone(), amb.reabrir(), amb.disco.clone(), 0.);
    coletor.abrir(FonteDeAudio::Mac, 0.).unwrap();
    amb.capturar(&mut coletor, &pcm_sintetico(&rot, 3.));
    coletor.encerrar(3.).unwrap();
    let c = rusqlite::Connection::open(&amb.banco).unwrap();
    inserir_trecho_raw(
        &c,
        &doc,
        &format!("{doc}|mac|0|1600"),
        0.10,
        0.80,
        "ligue amanhã",
    );
    for (inicio, fim) in [(5600i64, 8000i64), (32000, 44800)] {
        c.execute(
            "INSERT INTO lt_pendencias (id, documento, fonte, geracao, inicio, fim, motivo, estado) VALUES (?, ?, 'mac', 0, ?, ?, 'voz sem texto', 'aberta')",
            rusqlite::params![format!("{doc}|mac|0|{inicio}"), doc, inicio, fim],
        )
        .unwrap();
    }
    let oraculo = Oraculo::new(rot);
    let mut arm = amb.reabrir();
    ProcessadorDeReuniao::new(&doc, &mut arm, &amb.disco, &oraculo)
        .repetir_pendencias()
        .unwrap();
    assert_eq!(oraculo.chamadas(), 1);
    let pend = amb.armazem.pendencias(&doc).unwrap();
    let p1 = pend.iter().find(|p| p.inicio == 5600).unwrap();
    assert_eq!(p1.estado, "aberta");
    assert!(p1.motivo.contains("já tem texto"));
    assert_eq!(
        pend.iter().find(|p| p.inicio == 32000).unwrap().estado,
        "resolvida"
    );
    let textos: Vec<String> = amb
        .armazem
        .documento(&doc)
        .unwrap()
        .unwrap()
        .trechos
        .into_iter()
        .map(|t| t.texto)
        .collect();
    assert_eq!(textos, vec!["ligue amanhã", "depois"]);
}

#[test]
fn palavra_na_borda_da_pendencia_nao_retem() {
    let amb = Ambiente::new();
    let doc = amb.nova_reuniao();
    let rot = vec![
        PalavraReconhecida {
            texto: "antes".into(),
            inicio: 0.60,
            fim: 1.00,
        },
        PalavraReconhecida {
            texto: "depois".into(),
            inicio: 1.13,
            fim: 1.55,
        },
    ];
    let mut coletor = ColetorDeReuniao::new(doc.clone(), amb.reabrir(), amb.disco.clone(), 0.);
    coletor.abrir(FonteDeAudio::Mac, 0.).unwrap();
    amb.capturar(&mut coletor, &pcm_sintetico(&rot, 2.));
    coletor.encerrar(2.).unwrap();
    let c = rusqlite::Connection::open(&amb.banco).unwrap();
    inserir_trecho_raw(
        &c,
        &doc,
        &format!("{doc}|mac|0|18080"),
        1.13,
        1.55,
        "depois",
    );
    c.execute(
        "INSERT INTO lt_pendencias (id, documento, fonte, geracao, inicio, fim, motivo, estado) VALUES (?, ?, 'mac', 0, 8000, 19200, 'voz sem texto', 'aberta')",
        rusqlite::params![format!("{doc}|mac|0|8000"), doc],
    )
    .unwrap();
    let oraculo = Oraculo::new(rot);
    let mut arm = amb.reabrir();
    ProcessadorDeReuniao::new(&doc, &mut arm, &amb.disco, &oraculo)
        .repetir_pendencias()
        .unwrap();
    assert_eq!(amb.armazem.pendencias(&doc).unwrap()[0].estado, "resolvida");
    assert_eq!(palavras(&amb.armazem, &doc), vec!["antes", "depois"]);
}

#[test]
fn evento_ao_vivo_usa_id_persistido_na_colisao_entre_passos() {
    let amb = Ambiente::new();
    let doc = amb.nova_reuniao();
    let rot = vec![PalavraReconhecida {
        texto: "continuando".into(),
        inicio: 1.0,
        fim: 1.6,
    }];
    let c = rusqlite::Connection::open(&amb.banco).unwrap();
    let ocupado = format!("{doc}|mac|0|16000");
    inserir_trecho_raw(&c, &doc, &ocupado, 1.0, 1.2, "ok.");
    let mut coletor = ColetorDeReuniao::new(doc.clone(), amb.reabrir(), amb.disco.clone(), 0.);
    coletor.abrir(FonteDeAudio::Mac, 0.).unwrap();
    amb.capturar(&mut coletor, &pcm_sintetico(&rot, 3.));
    coletor.encerrar(3.).unwrap();
    let oraculo = Oraculo::new(rot);
    let mut arm = amb.reabrir();
    let evento_ids: Vec<String> = ProcessadorDeReuniao::new(&doc, &mut arm, &amb.disco, &oraculo)
        .fechar()
        .unwrap()
        .into_iter()
        .map(|t| t.id)
        .collect();
    let mut s = c
        .prepare("SELECT id FROM lt_trechos WHERE documento = ? AND texto = 'continuando'")
        .unwrap();
    let no_banco: Vec<String> = s
        .query_map([&doc], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(no_banco, vec![format!("{ocupado}#1")]);
    assert_eq!(evento_ids, no_banco);
}

#[test]
fn orfao_atrasado_com_pendencia_curta_aberta_ou_resolvida_nao_perde_fala() {
    for estado in ["aberta", "resolvida"] {
        let amb = Ambiente::new();
        let doc = amb.nova_reuniao();
        let rot = vec![
            PalavraReconhecida {
                texto: "primeira".into(),
                inicio: 0.10,
                fim: 0.20,
            },
            PalavraReconhecida {
                texto: "segunda".into(),
                inicio: 0.48,
                fim: 0.80,
            },
        ];
        let c = rusqlite::Connection::open(&amb.banco).unwrap();
        c.execute("INSERT INTO lt_costuras (documento, fonte, geracao, confirmado_ate, fechada) VALUES (?, 'mac', 0, 16000, 1)", [&doc])
            .unwrap();
        c.execute(
            "INSERT INTO lt_pendencias (id, documento, fonte, geracao, inicio, fim, motivo, estado) VALUES (?, ?, 'mac', 0, 0, 4800, 'antiga', ?)",
            rusqlite::params![format!("{doc}|mac|0|0"), doc, estado],
        )
        .unwrap();
        amb.disco
            .gravar(
                IdBloco {
                    documento: doc.clone(),
                    fonte: FonteDeAudio::Mac,
                    geracao: 0,
                    seq: 0,
                },
                0,
                &pcm_sintetico(&rot, 1.),
            )
            .unwrap();
        let mut reaberto = amb.reabrir();
        assert_eq!(
            reaberto.preparar_abertura(&amb.disco).adotados,
            1,
            "{estado}"
        );
        let oraculo = Oraculo::new(rot);
        let id = Identificador(vec![FaixaDeVoz {
            voz: "pessoa-1".into(),
            inicio: 0.,
            fim: 1.,
        }]);
        {
            let mut p = ProcessadorDeReuniao::new(&doc, &mut reaberto, &amb.disco, &oraculo);
            p.recuperar().unwrap();
            p.encerrar_costuras().unwrap();
            assert!(p.passada_final(Some(&id)).unwrap(), "{estado}");
        }
        assert_eq!(
            palavras(&reaberto, &doc),
            vec!["primeira", "segunda"],
            "{estado}"
        );
    }
}

/// Link para pasta. No Windows usa junção (`mklink /J`), que não exige privilégio e é o
/// ponto de reparse que um programa comum consegue criar.
fn ligar_pasta(alvo: &Path, link: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(alvo, link).unwrap();
    #[cfg(windows)]
    {
        let ok = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(alvo)
            .status()
            .unwrap()
            .success();
        assert!(ok, "mklink /J falhou");
    }
}

/// Link simbólico para arquivo. No Windows exige privilégio ou modo de desenvolvedor;
/// sem eles devolve `false` e o teste avisa que não rodou essa parte.
fn ligar_arquivo(alvo: &Path, link: &Path) -> bool {
    #[cfg(unix)]
    let r = std::os::unix::fs::symlink(alvo, link);
    #[cfg(windows)]
    let r = std::os::windows::fs::symlink_file(alvo, link);
    match r {
        Ok(()) => true,
        Err(e)
            if e.kind() == std::io::ErrorKind::PermissionDenied
                || e.raw_os_error() == Some(1314) =>
        {
            eprintln!("link simbólico de arquivo sem privilégio neste sistema: {e}");
            false
        }
        Err(e) => panic!("{e}"),
    }
}

#[test]
fn limpeza_nao_segue_link_no_lugar_da_pasta_da_reuniao() {
    let amb = Ambiente::new();
    let doc = novo_uuid();
    let externa = amb.tmp.path().join("externa");
    fs::create_dir_all(&externa).unwrap();
    let alvo = externa.join("mac_0_0_0_1_x.f32");
    fs::write(&alvo, vec![0u8; 4]).unwrap();
    fs::create_dir_all(&amb.disco.raiz).unwrap();
    ligar_pasta(&externa, &amb.disco.raiz.join(&doc));
    assert!(amb
        .disco
        .apagar(&doc, &format!("{doc}/mac_0_0_0_1_x.f32"))
        .is_err());
    amb.disco.remover_pasta(&doc);
    assert!(alvo.exists());
    assert!(!DiscoDeBlocos::arquivo_valido(
        &format!("{doc}/a.f32"),
        &novo_uuid()
    ));
}

#[test]
fn limpeza_final_nao_apaga_arquivo_de_outra_reuniao() {
    let amb = Ambiente::new();
    let doc = amb.nova_reuniao();
    let outra = amb
        .disco
        .gravar(
            IdBloco {
                documento: novo_uuid(),
                fonte: FonteDeAudio::Mac,
                geracao: 0,
                seq: 0,
            },
            0,
            &[0.4],
        )
        .unwrap();
    let c = rusqlite::Connection::open(&amb.banco).unwrap();
    c.execute(
        "INSERT INTO lt_blocos (id, documento, fonte, geracao, seq, inicio, quantidade, arquivo, tamanho, soma, estado, tentativas, audio_liberado) \
         VALUES (?, ?, 'mac', 0, 0, 0, 1, ?, 4, NULL, 'transcrito', 0, 1)",
        rusqlite::params![format!("{doc}|mac|0|0"), doc, outra.arquivo.as_deref().unwrap()],
    )
    .unwrap();
    let oraculo = Oraculo::new(vec![]);
    let mut arm = amb.reabrir();
    ProcessadorDeReuniao::new(&doc, &mut arm, &amb.disco, &oraculo).limpar_arquivos();
    assert!(amb.disco.existe(&outra));
}

#[test]
fn apagar_so_aceita_arquivo_de_uma_reuniao() {
    let amb = Ambiente::new();
    let doc = novo_uuid();
    let b = amb
        .disco
        .gravar(
            IdBloco {
                documento: doc.clone(),
                fonte: FonteDeAudio::Mac,
                geracao: 0,
                seq: 0,
            },
            0,
            &[0.1, 0.2],
        )
        .unwrap();
    let fora = amb.tmp.path().join("fora.f32");
    fs::write(&fora, [1u8]).unwrap();
    for invalido in [
        "".to_string(),
        "../fora.f32".into(),
        format!("{doc}/../../fora.f32"),
        doc.clone(),
        "nao-uuid/x.f32".into(),
        format!("{doc}/x.txt"),
    ] {
        assert!(amb.disco.apagar(&doc, &invalido).is_err(), "{invalido}");
    }
    amb.disco.remover_pasta("");
    amb.disco.remover_pasta("..");
    assert!(fora.exists());
    assert!(amb.disco.existe(&b));
    amb.disco
        .apagar(&doc, b.arquivo.as_deref().unwrap())
        .unwrap();
    assert!(!amb.disco.existe(&b));
}

#[test]
fn limpeza_de_abertura_ignora_caminho_invalido_no_banco() {
    let amb = Ambiente::new();
    let doc = amb.nova_reuniao();
    let vitima = amb
        .disco
        .gravar(
            IdBloco {
                documento: novo_uuid(),
                fonte: FonteDeAudio::Mac,
                geracao: 0,
                seq: 0,
            },
            0,
            &[0.3],
        )
        .unwrap();
    let c = rusqlite::Connection::open(&amb.banco).unwrap();
    c.execute(
        "INSERT INTO lt_blocos (id, documento, fonte, geracao, seq, inicio, quantidade, arquivo, tamanho, soma, estado, tentativas, audio_liberado) \
         VALUES (?, ?, 'mac', 0, 0, 0, 1, '..', 4, NULL, 'transcrito', 0, 1)",
        rusqlite::params![format!("{doc}|mac|0|0"), doc],
    )
    .unwrap();
    let mut reaberto = amb.reabrir();
    let r = reaberto.preparar_abertura(&amb.disco);
    assert!(r.falhas.is_empty(), "{:?}", r.falhas);
    assert!(amb.disco.existe(&vitima));
    assert!(amb.disco.raiz.exists());
}

#[test]
fn adocao_ignora_link_simbolico_e_remove_wav_de_identificacao_esquecido() {
    let amb = Ambiente::new();
    let doc = novo_uuid();
    let pasta = amb.disco.pasta(&doc).unwrap();
    let externo = amb.tmp.path().join("externo.bin");
    fs::write(&externo, vec![0u8; 8]).unwrap();
    let nome_link = format!("mac_0_0_0_2_{}.f32", disco_de_blocos::soma(&[0u8; 8]));
    let link = pasta.join(&nome_link);
    let tem_link = ligar_arquivo(&externo, &link);
    let wav = pasta.join(format!(
        "{}{}.wav",
        disco_de_blocos::PREFIXO_WAV,
        novo_uuid()
    ));
    fs::write(&wav, vec![0u8; 44]).unwrap();
    let mut reaberto = amb.reabrir();
    let r = reaberto.preparar_abertura(&amb.disco);
    assert_eq!(r.adotados, 0);
    if tem_link {
        assert!(r.invalidos.contains(&format!("{doc}/{nome_link}")));
    }
    assert!(!wav.exists());
    assert!(externo.exists());
}

#[cfg(unix)]
#[test]
fn banco_e_pasta_de_audio_e_pcm_so_do_usuario() {
    use std::os::unix::fs::PermissionsExt;
    let amb = Ambiente::new();
    let doc = novo_uuid();
    let b = amb
        .disco
        .gravar(
            IdBloco {
                documento: doc.clone(),
                fonte: FonteDeAudio::Mac,
                geracao: 0,
                seq: 0,
            },
            0,
            &[0.1],
        )
        .unwrap();
    let modo = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(modo(&amb.banco), 0o600);
    assert_eq!(modo(&amb.disco.raiz), 0o700);
    assert_eq!(modo(&amb.disco.raiz.join(b.arquivo.unwrap())), 0o600);
}

/// Pasta do usuário com acento (por exemplo `C:\\Users\\Vinícius\\AppData`): o banco e o PCM
/// precisam abrir, gravar, reabrir e ler pelo mesmo caminho.
#[test]
fn banco_e_audio_em_caminho_com_acento() {
    let tmp = comum::tempdir();
    let raiz = tmp.path().join("Usuário Ção").join("Dita.se ñ");
    let banco = raiz.join("lançamento.sqlite");
    let armazem = ArmazemLancamento::new(&banco).unwrap();
    let disco = DiscoDeBlocos::new(raiz.join("Áudio"));
    let doc = novo_uuid();
    armazem
        .criar_documento(&doc, "reuniao", "Reunião", Some("mac"), "gravando")
        .unwrap();
    let b = disco
        .gravar(
            IdBloco {
                documento: doc.clone(),
                fonte: FonteDeAudio::Mac,
                geracao: 0,
                seq: 0,
            },
            0,
            &[0.25, -0.5],
        )
        .unwrap();
    assert!(armazem.registrar(&b).unwrap());
    drop(armazem);
    let reaberto = ArmazemLancamento::new(&banco).unwrap();
    let blocos = reaberto.blocos(&doc).unwrap();
    assert_eq!(blocos.len(), 1);
    assert_eq!(disco.ler(&blocos[0]).unwrap(), vec![0.25, -0.5]);
    let wav = raiz
        .join("Áudio")
        .join(&doc)
        .join(format!("{}ç.wav", disco_de_blocos::PREFIXO_WAV));
    disco.escrever_wav(&[&blocos[0]], &wav).unwrap();
    assert_eq!(fs::metadata(&wav).unwrap().len(), 44 + 8);
    fs::remove_file(&wav).unwrap();
    disco
        .apagar(&doc, blocos[0].arquivo.as_deref().unwrap())
        .unwrap();
    assert!(!disco.existe(&blocos[0]));
}

#[test]
fn abertura_e_limpeza_recusam_juncao_no_ancestral_da_raiz() {
    let tmp = comum::tempdir();
    let externa = tmp.path().join("externa");
    let destino_real = DiscoDeBlocos::new(externa.join("Audio"));
    fs::create_dir(&externa).unwrap();
    let doc = novo_uuid();
    let bloco = destino_real
        .gravar(
            IdBloco {
                documento: doc.clone(),
                fonte: FonteDeAudio::Mac,
                geracao: 0,
                seq: 0,
            },
            0,
            &[0.1; 320],
        )
        .unwrap();
    let pai = tmp.path().join("redirecionada");
    ligar_pasta(&externa, &pai);
    let disco = DiscoDeBlocos::new(pai.join("Audio"));
    let mut a = ArmazemLancamento::new(tmp.path().join("teste.sqlite")).unwrap();
    let r = a.preparar_abertura(&disco);
    assert_eq!(r.adotados, 0);
    assert_eq!(r.falhas.len(), 1);
    assert!(!a.existe_documento(&doc).unwrap());
    assert!(disco.pasta(&doc).is_err());
    disco.remover_pasta(&doc);
    assert!(destino_real.existe(&bloco));
    assert_eq!(destino_real.ler(&bloco).unwrap(), vec![0.1; 320]);
}
