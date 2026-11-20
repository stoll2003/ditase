use nucleo::{motores::*, *};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

const TAXA_USIZE: usize = TAXA as usize;

fn novo_uuid() -> String {
    static N: AtomicU64 = AtomicU64::new(1);
    let n = N.fetch_add(1, Ordering::Relaxed);
    format!("{n:08x}-0000-4000-8000-000000000000")
}

struct Oraculo {
    roteiro: Vec<PalavraReconhecida>,
    vazias_restantes: AtomicUsize,
    chamadas: AtomicUsize,
}

impl Oraculo {
    fn new(roteiro: Vec<PalavraReconhecida>) -> Self {
        Self::com_vazias(roteiro, 0)
    }
    fn com_vazias(roteiro: Vec<PalavraReconhecida>, vazias: usize) -> Self {
        Self {
            roteiro,
            vazias_restantes: AtomicUsize::new(vazias),
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
        let mut cur = self.vazias_restantes.load(Ordering::Relaxed);
        while cur > 0 {
            if self
                .vazias_restantes
                .compare_exchange(cur, cur - 1, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                return Ok(vec![]);
            }
            cur = self.vazias_restantes.load(Ordering::Relaxed);
        }
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

struct Identificador {
    faixas: Vec<FaixaDeVoz>,
    falhar: bool,
}

impl IdentificadorDeFalantes for Identificador {
    fn faixas(&self, wav: &Path) -> Result<Vec<FaixaDeVoz>, Erro> {
        if self.falhar {
            return Err("identificador indisponível".into());
        }
        assert!(wav.exists());
        Ok(self.faixas.clone())
    }
}

fn roteiro(textos: &[&str], inicio: f64, duracao: f64, pausa: f64) -> Vec<PalavraReconhecida> {
    let mut t = inicio;
    textos
        .iter()
        .map(|texto| {
            let p = PalavraReconhecida {
                texto: (*texto).into(),
                inicio: t,
                fim: t + duracao,
            };
            t += duracao + pausa;
            p
        })
        .collect()
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
    _tmp: tempfile::TempDir,
    banco: PathBuf,
    armazem: ArmazemLancamento,
    disco: DiscoDeBlocos,
}

impl Ambiente {
    fn new() -> Self {
        Self::com_escrita(None)
    }
    fn com_escrita(escrita: Option<Escrita>) -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let banco = tmp.path().join("teste.sqlite");
        let armazem = ArmazemLancamento::new(&banco).unwrap();
        let audio = tmp.path().join("Audio");
        let disco = match escrita {
            Some(e) => DiscoDeBlocos::com_escrita(audio, e),
            None => DiscoDeBlocos::new(audio),
        };
        Self {
            _tmp: tmp,
            banco,
            armazem,
            disco,
        }
    }
    fn reabrir(&self) -> ArmazemLancamento {
        ArmazemLancamento::new(&self.banco).unwrap()
    }
    fn nova_reuniao(&self, fonte: &str) -> String {
        let id = novo_uuid();
        self.armazem
            .criar_documento(&id, "reuniao", "Reunião sintética", Some(fonte), "gravando")
            .unwrap();
        id
    }
    fn capturar(
        &self,
        coletor: &mut ColetorDeReuniao,
        fonte: FonteDeAudio,
        pcm: &[f32],
        desde: f64,
        mut proc: Option<&mut ProcessadorDeReuniao<'_>>,
    ) {
        let passo = TAXA_USIZE / 10;
        let mut i = 0;
        while i < pcm.len() {
            let fim = pcm.len().min(i + passo);
            coletor
                .receber(fonte, desde + i as f64 / f64::from(TAXA), &pcm[i..fim])
                .unwrap();
            if let Some(p) = proc.as_deref_mut() {
                if (i / passo) % 10 == 9 {
                    p.bloco_selado().unwrap();
                }
            }
            i += passo;
        }
    }
    fn textos(&self, doc: &str) -> Vec<String> {
        self.armazem
            .documento(doc)
            .unwrap()
            .map(|d| d.trechos.into_iter().map(|t| t.texto).collect())
            .unwrap_or_default()
    }
    fn palavras(&self, doc: &str) -> Vec<String> {
        self.textos(doc)
            .into_iter()
            .flat_map(|t| t.split_whitespace().map(str::to_owned).collect::<Vec<_>>())
            .collect()
    }
}

fn capturada(
    amb: &Ambiente,
    segundos: f64,
    fonte: FonteDeAudio,
    rot: &[PalavraReconhecida],
) -> String {
    let doc = amb.nova_reuniao("mac");
    let mut coletor = ColetorDeReuniao::new(doc.clone(), amb.reabrir(), amb.disco.clone(), 0.);
    coletor.abrir(fonte, 0.).unwrap();
    amb.capturar(&mut coletor, fonte, &pcm_sintetico(rot, segundos), 0., None);
    coletor.encerrar(segundos).unwrap();
    doc
}

fn ao_vivo(rot: &[PalavraReconhecida], segundos: f64, oraculo: &Oraculo) -> (Ambiente, String) {
    let amb = Ambiente::new();
    let doc = amb.nova_reuniao("mac");
    let mut coletor = ColetorDeReuniao::new(doc.clone(), amb.reabrir(), amb.disco.clone(), 0.);
    let mut arm_proc = amb.reabrir();
    let mut proc = ProcessadorDeReuniao::new(&doc, &mut arm_proc, &amb.disco, oraculo);
    coletor.abrir(FonteDeAudio::Mac, 0.).unwrap();
    amb.capturar(
        &mut coletor,
        FonteDeAudio::Mac,
        &pcm_sintetico(rot, segundos),
        0.,
        Some(&mut proc),
    );
    coletor.encerrar(segundos).unwrap();
    proc.fechar().unwrap();
    (amb, doc)
}

fn roteiro_padrao() -> Vec<PalavraReconhecida> {
    roteiro(
        &["primeira", "fala", "da", "reunião", "sintética"],
        0.4,
        0.4,
        0.35,
    )
}

// MARK: Garantia 1

#[test]
fn queda_depois_de_gravar_e_antes_de_registrar_adota_o_arquivo() {
    let amb = Ambiente::new();
    let doc = novo_uuid();
    let rot = roteiro_padrao();
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
    let pasta = amb.disco.pasta(&doc).unwrap();
    let tmp = pasta.join("mac_0_1_16000_16000.f32.tmp");
    fs::write(&tmp, vec![0u8; 100]).unwrap();
    let mut reaberto = amb.reabrir();
    let r = reaberto.preparar_abertura(&amb.disco);
    assert_eq!(r.adotados, 1);
    assert_eq!(
        reaberto
            .blocos(&doc)
            .unwrap()
            .into_iter()
            .map(|b| b.estado)
            .collect::<Vec<_>>(),
        vec![EstadoBloco::Capturado]
    );
    assert!(reaberto
        .recuperaveis(None)
        .unwrap()
        .iter()
        .any(|d| d.id == doc));
    assert!(!tmp.exists());
    assert_eq!(reaberto.preparar_abertura(&amb.disco).adotados, 0);
}

#[test]
fn adocao_de_varios_orfaos_comeca_no_primeiro_bloco() {
    let amb = Ambiente::new();
    let doc = novo_uuid();
    let falas = ["primeira", "segunda", "terceira", "quarta", "quinta"];
    let rot: Vec<_> = falas
        .iter()
        .enumerate()
        .map(|(i, t)| PalavraReconhecida {
            texto: (*t).into(),
            inicio: i as f64 + 0.2,
            fim: i as f64 + 0.6,
        })
        .collect();
    let pcm = pcm_sintetico(&rot, 5.);
    for seq in (0..5).rev() {
        let a = seq * TAXA_USIZE;
        amb.disco
            .gravar(
                IdBloco {
                    documento: doc.clone(),
                    fonte: FonteDeAudio::Mac,
                    geracao: 0,
                    seq: seq as u64,
                },
                a,
                &pcm[a..a + TAXA_USIZE],
            )
            .unwrap();
    }
    let mut reaberto = amb.reabrir();
    assert_eq!(reaberto.preparar_abertura(&amb.disco).adotados, 5);
    assert_eq!(reaberto.costuras(&doc).unwrap()[0].confirmado_ate, 0);
    let oraculo = Oraculo::new(rot);
    let id = Identificador {
        faixas: vec![FaixaDeVoz {
            voz: "pessoa-1".into(),
            inicio: 0.,
            fim: 5.,
        }],
        falhar: false,
    };
    {
        let mut p = ProcessadorDeReuniao::new(&doc, &mut reaberto, &amb.disco, &oraculo);
        p.recuperar().unwrap();
        p.encerrar_costuras().unwrap();
        assert!(p.passada_final(Some(&id)).unwrap());
    }
    let palavras: Vec<String> = reaberto
        .documento(&doc)
        .unwrap()
        .unwrap()
        .trechos
        .into_iter()
        .flat_map(|t| {
            t.texto
                .split_whitespace()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(palavras, falas);
}

#[test]
fn trechos_com_mesmo_inicio_nao_colidem() {
    let amb = Ambiente::new();
    let rot = vec![
        PalavraReconhecida {
            texto: "ok.".into(),
            inicio: 1.0,
            fim: 1.15,
        },
        PalavraReconhecida {
            texto: "continuando".into(),
            inicio: 1.0,
            fim: 1.5,
        },
    ];
    let doc = capturada(&amb, 3., FonteDeAudio::Mac, &rot);
    let oraculo = Oraculo::new(rot.clone());
    let mut arm = amb.reabrir();
    ProcessadorDeReuniao::new(&doc, &mut arm, &amb.disco, &oraculo)
        .fechar()
        .unwrap();
    let textos: HashSet<String> = amb.textos(&doc).into_iter().collect();
    assert_eq!(textos, HashSet::from(["ok.".into(), "continuando".into()]));
    let ids: Vec<String> = Costura::trechos(&rot, &doc, FonteDeAudio::Mac, 0, None)
        .into_iter()
        .map(|t| t.id)
        .collect();
    assert_eq!(ids.iter().collect::<HashSet<_>>().len(), 2);
    assert_eq!(
        ids,
        Costura::trechos(&rot, &doc, FonteDeAudio::Mac, 0, None)
            .into_iter()
            .map(|t| t.id)
            .collect::<Vec<_>>()
    );
}

#[test]
fn inserir_trecho_com_id_ocupado_ganha_sufixo_e_repeticao_nao_duplica() {
    let mut amb = Ambiente::new();
    let doc = amb.nova_reuniao("mac");
    amb.armazem
        .abrir_costura(&doc, FonteDeAudio::Mac, 0, 0)
        .unwrap();
    amb.armazem
        .tomar_costura(&doc, FonteDeAudio::Mac, 0, "d1")
        .unwrap();
    let a = TrechoNovo {
        id: format!("{doc}|mac|0|16000"),
        fonte: FonteDeAudio::Mac,
        inicio: 1.0,
        fim: 1.2,
        texto: "ok.".into(),
        voz: "chamada".into(),
    };
    let b = TrechoNovo {
        texto: "continuando".into(),
        fim: 1.6,
        ..a.clone()
    };
    let persistidos = amb
        .armazem
        .confirmar_persistindo(&Passo {
            documento: doc.clone(),
            fonte: FonteDeAudio::Mac,
            geracao: 0,
            dono: "d1".into(),
            esperado: 0,
            novo: 32000,
            trechos: vec![a.clone(), b, a.clone()],
            com_atividade: HashSet::new(),
            pendencia: None,
            fechar: false,
        })
        .unwrap()
        .unwrap();
    let ids: Vec<String> = persistidos.into_iter().map(|t| t.id).collect();
    assert_eq!(ids, vec![a.id.clone(), format!("{}#1", a.id)]);
}

#[test]
fn registro_com_arquivo_ausente_vira_lacuna_visivel() {
    let amb = Ambiente::new();
    let doc = capturada(&amb, 3., FonteDeAudio::Mac, &roteiro_padrao());
    let bloco = amb
        .armazem
        .blocos(&doc)
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    amb.disco.remover(bloco.arquivo.as_deref().unwrap());
    let mut reaberto = amb.reabrir();
    assert_eq!(reaberto.preparar_abertura(&amb.disco).lacunas, 1);
    assert_eq!(
        reaberto.blocos(&doc).unwrap()[0].estado,
        EstadoBloco::Lacuna
    );
    assert!(reaberto.documento(&doc).unwrap().unwrap().lacunas.is_some());
}

#[test]
fn soma_divergente_na_leitura_vira_lacuna() {
    let amb = Ambiente::new();
    let rot = roteiro_padrao();
    let doc = capturada(&amb, 2., FonteDeAudio::Mac, &rot);
    let bloco = amb
        .armazem
        .blocos(&doc)
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    let url = amb.disco.raiz.join(bloco.arquivo.as_deref().unwrap());
    let mut dados = fs::read(&url).unwrap();
    dados[0] ^= 0xFF;
    fs::write(&url, dados).unwrap();
    let oraculo = Oraculo::new(rot);
    let mut arm = amb.reabrir();
    ProcessadorDeReuniao::new(&doc, &mut arm, &amb.disco, &oraculo)
        .recuperar()
        .unwrap();
    assert_eq!(
        amb.armazem.blocos(&doc).unwrap()[0].estado,
        EstadoBloco::Lacuna
    );
}

#[test]
fn queda_entre_registro_e_commit_recupera_pelo_mesmo_caminho() {
    let amb = Ambiente::new();
    let rot = roteiro_padrao();
    let interrompida = capturada(&amb, 5., FonteDeAudio::Mac, &rot);
    assert!(amb
        .armazem
        .blocos(&interrompida)
        .unwrap()
        .iter()
        .all(|b| b.estado == EstadoBloco::Capturado));
    let oraculo = Oraculo::new(rot.clone());
    let mut r1 = amb.reabrir();
    ProcessadorDeReuniao::new(&interrompida, &mut r1, &amb.disco, &oraculo)
        .recuperar()
        .unwrap();

    let ao_vivo_doc = capturada(&amb, 5., FonteDeAudio::Mac, &rot);
    let mut r2 = amb.reabrir();
    ProcessadorDeReuniao::new(&ao_vivo_doc, &mut r2, &amb.disco, &oraculo)
        .fechar()
        .unwrap();
    assert_eq!(
        amb.palavras(&interrompida),
        rot.iter().map(|p| p.texto.clone()).collect::<Vec<_>>()
    );
    assert_eq!(amb.textos(&interrompida), amb.textos(&ao_vivo_doc));
}

#[test]
fn queda_depois_do_commit_e_antes_da_limpeza_repete_a_limpeza() {
    let mut amb = Ambiente::new();
    let rot = roteiro_padrao();
    let doc = capturada(&amb, 5., FonteDeAudio::Mac, &rot);
    let oraculo = Oraculo::new(rot.clone());
    {
        let mut arm = amb.reabrir();
        ProcessadorDeReuniao::new(&doc, &mut arm, &amb.disco, &oraculo)
            .fechar()
            .unwrap();
    }
    assert!(amb
        .armazem
        .gravar_falantes(&doc, &HashMap::new(), true, true)
        .unwrap());
    let blocos = amb.armazem.blocos(&doc).unwrap();
    assert!(blocos.iter().all(|b| amb.disco.existe(b)));
    let mut reaberto = amb.reabrir();
    reaberto.preparar_abertura(&amb.disco);
    assert!(blocos.iter().all(|b| !amb.disco.existe(b)));
    assert!(reaberto
        .blocos(&doc)
        .unwrap()
        .iter()
        .all(|b| b.arquivo.is_none()));
    assert_eq!(
        amb.palavras(&doc),
        rot.iter().map(|p| p.texto.clone()).collect::<Vec<_>>()
    );
}

#[test]
fn transacao_exige_estado_e_tentativa_esperados() {
    let mut amb = Ambiente::new();
    let rot = roteiro_padrao();
    let doc = capturada(&amb, 5., FonteDeAudio::Mac, &rot);
    let oraculo = Oraculo::new(rot.clone());
    {
        let mut arm = amb.reabrir();
        ProcessadorDeReuniao::new(&doc, &mut arm, &amb.disco, &oraculo)
            .fechar()
            .unwrap();
    }
    let ok = amb
        .armazem
        .confirmar(&Passo {
            documento: doc.clone(),
            fonte: FonteDeAudio::Mac,
            geracao: 0,
            dono: "outro".into(),
            esperado: 0,
            novo: 16000,
            trechos: vec![TrechoNovo {
                id: "x".into(),
                fonte: FonteDeAudio::Mac,
                inicio: 0.,
                fim: 1.,
                texto: "duplicado".into(),
                voz: "chamada".into(),
            }],
            com_atividade: HashSet::new(),
            pendencia: None,
            fechar: false,
        })
        .unwrap();
    assert!(!ok);
    let mut arm2 = amb.reabrir();
    ProcessadorDeReuniao::new(&doc, &mut arm2, &amb.disco, &oraculo)
        .recuperar()
        .unwrap();
    assert_eq!(
        amb.palavras(&doc),
        rot.iter().map(|p| p.texto.clone()).collect::<Vec<_>>()
    );
}

// MARK: Descarte

#[test]
fn descartar_grava_estado_terminal_antes_da_limpeza() {
    let mut amb = Ambiente::new();
    let rot = roteiro_padrao();
    let doc = capturada(&amb, 5., FonteDeAudio::Mac, &rot);
    let arquivos: Vec<String> = amb
        .armazem
        .blocos(&doc)
        .unwrap()
        .into_iter()
        .filter_map(|b| b.arquivo)
        .collect();
    assert!(amb.armazem.descartar(&doc).unwrap());
    assert!(amb
        .armazem
        .blocos(&doc)
        .unwrap()
        .iter()
        .all(|b| b.estado == EstadoBloco::Descartado));
    assert!(arquivos.iter().all(|a| amb.disco.raiz.join(a).exists()));
    let oraculo = Oraculo::new(rot);
    {
        let mut arm = amb.reabrir();
        let _ = ProcessadorDeReuniao::new(&doc, &mut arm, &amb.disco, &oraculo).recuperar();
    }
    assert!(amb.textos(&doc).is_empty());
    let mut reaberto = amb.reabrir();
    reaberto.preparar_abertura(&amb.disco);
    assert!(arquivos.iter().all(|a| !amb.disco.raiz.join(a).exists()));
    assert!(!reaberto
        .recuperaveis(None)
        .unwrap()
        .iter()
        .any(|d| d.id == doc));
    assert!(!reaberto
        .lista("reuniao")
        .unwrap()
        .iter()
        .any(|d| d.id == doc));
}

#[test]
fn descartar_preserva_fala_confirmada_e_oculta_so_o_pendente() {
    let mut amb = Ambiente::new();
    let rot = roteiro_padrao();
    let doc = capturada(&amb, 5., FonteDeAudio::Mac, &rot);
    let oraculo = Oraculo::new(rot.clone());
    {
        let mut arm = amb.reabrir();
        ProcessadorDeReuniao::new(&doc, &mut arm, &amb.disco, &oraculo)
            .fechar()
            .unwrap();
    }
    assert!(amb.armazem.descartar(&doc).unwrap());
    assert_eq!(
        amb.palavras(&doc),
        rot.iter().map(|p| p.texto.clone()).collect::<Vec<_>>()
    );
    assert!(amb
        .armazem
        .lista("reuniao")
        .unwrap()
        .iter()
        .any(|d| d.id == doc));
}

// MARK: Garantia 4 e edições

#[test]
fn pcm_retido_ate_passada_final_e_edicoes_prevalecem() {
    let mut amb = Ambiente::new();
    let tres = vec![
        PalavraReconhecida {
            texto: "um.".into(),
            inicio: 0.3,
            fim: 0.7,
        },
        PalavraReconhecida {
            texto: "dois.".into(),
            inicio: 2.0,
            fim: 2.4,
        },
        PalavraReconhecida {
            texto: "três.".into(),
            inicio: 3.7,
            fim: 4.1,
        },
    ];
    let doc = capturada(&amb, 5., FonteDeAudio::Mac, &tres);
    let oraculo = Oraculo::new(tres);
    {
        let mut arm = amb.reabrir();
        ProcessadorDeReuniao::new(&doc, &mut arm, &amb.disco, &oraculo)
            .fechar()
            .unwrap();
    }
    assert!(amb
        .armazem
        .blocos(&doc)
        .unwrap()
        .iter()
        .all(|b| amb.disco.existe(b)));
    let antes = amb.armazem.documento(&doc).unwrap().unwrap();
    assert_eq!(
        antes
            .vozes
            .iter()
            .map(|v| v.nome.as_str())
            .collect::<HashSet<_>>(),
        HashSet::from(["Chamada"])
    );
    let editado = antes.trechos[0].id.clone();
    amb.armazem.reatribuir(&doc, &editado, "voce").unwrap();

    let faixas = vec![
        FaixaDeVoz {
            voz: "pessoa-a".into(),
            inicio: 0.,
            fim: 1.5,
        },
        FaixaDeVoz {
            voz: "pessoa-b".into(),
            inicio: 1.5,
            fim: 3.0,
        },
        FaixaDeVoz {
            voz: "pessoa-a".into(),
            inicio: 3.0,
            fim: 5.0,
        },
    ];
    let id = Identificador {
        faixas,
        falhar: false,
    };
    {
        let mut arm = amb.reabrir();
        assert!(
            ProcessadorDeReuniao::new(&doc, &mut arm, &amb.disco, &oraculo)
                .passada_final(Some(&id))
                .unwrap()
        );
    }
    let depois = amb.armazem.documento(&doc).unwrap().unwrap();
    assert_eq!(
        depois.trechos.iter().map(|t| &t.texto).collect::<Vec<_>>(),
        antes.trechos.iter().map(|t| &t.texto).collect::<Vec<_>>()
    );
    assert_eq!(
        depois.trechos.iter().find(|t| t.id == editado).unwrap().voz,
        "voce"
    );
    assert!(depois.vozes.iter().any(|v| v.nome == "Pessoa 1"));
    assert!(amb
        .armazem
        .blocos(&doc)
        .unwrap()
        .iter()
        .all(|b| b.arquivo.is_none()));

    let pessoas: Vec<_> = depois
        .vozes
        .iter()
        .filter(|v| v.id.starts_with("pessoa"))
        .collect();
    assert_eq!(pessoas.len(), 2);
    amb.armazem.renomear(&doc, &pessoas[0].id, "Ana").unwrap();
    amb.armazem.renomear(&doc, &pessoas[1].id, "ana").unwrap();
    let final_doc = amb.armazem.documento(&doc).unwrap().unwrap();
    assert_eq!(
        final_doc
            .vozes
            .iter()
            .filter(|v| v.id != "voce")
            .map(|v| v.nome.as_str())
            .collect::<Vec<_>>(),
        vec!["Ana"]
    );
    assert_eq!(
        final_doc
            .trechos
            .iter()
            .map(|t| &t.texto)
            .collect::<Vec<_>>(),
        antes.trechos.iter().map(|t| &t.texto).collect::<Vec<_>>()
    );
}

#[test]
fn passada_final_indisponivel_mantem_audio_para_nova_tentativa() {
    let amb = Ambiente::new();
    let rot = roteiro_padrao();
    let doc = capturada(&amb, 5., FonteDeAudio::Mac, &rot);
    let oraculo = Oraculo::new(rot);
    let mut arm = amb.reabrir();
    let mut p = ProcessadorDeReuniao::new(&doc, &mut arm, &amb.disco, &oraculo);
    p.fechar().unwrap();
    let falha = Identificador {
        faixas: vec![],
        falhar: true,
    };
    assert!(!p.passada_final(Some(&falha)).unwrap());
    assert!(amb
        .armazem
        .blocos(&doc)
        .unwrap()
        .iter()
        .all(|b| amb.disco.existe(b)));
    assert!(amb
        .armazem
        .recuperaveis(None)
        .unwrap()
        .iter()
        .any(|d| d.id == doc));
    let ok = Identificador {
        faixas: vec![FaixaDeVoz {
            voz: "x".into(),
            inicio: 0.,
            fim: 5.,
        }],
        falhar: false,
    };
    assert!(p.passada_final(Some(&ok)).unwrap());
    assert!(!amb
        .armazem
        .recuperaveis(None)
        .unwrap()
        .iter()
        .any(|d| d.id == doc));
}

#[test]
fn edicoes_feitas_antes_da_recuperacao_continuam_depois() {
    let mut amb = Ambiente::new();
    let rot = roteiro_padrao();
    let doc = capturada(&amb, 5., FonteDeAudio::Mac, &rot);
    let oraculo = Oraculo::new(rot.clone());
    {
        let mut arm = amb.reabrir();
        ProcessadorDeReuniao::new(&doc, &mut arm, &amb.disco, &oraculo)
            .bloco_selado()
            .unwrap();
    }
    let parcial = amb.armazem.documento(&doc).unwrap().unwrap();
    let alvo = parcial.trechos[0].id.clone();
    amb.armazem
        .atribuir_nova_pessoa(&doc, &alvo, "Bruno")
        .unwrap();
    amb.armazem
        .definir(&doc, CampoDocumento::Nome("Planejamento".into()))
        .unwrap();
    {
        let mut arm = amb.reabrir();
        ProcessadorDeReuniao::new(&doc, &mut arm, &amb.disco, &oraculo)
            .recuperar()
            .unwrap();
    }
    let recuperada = amb.armazem.documento(&doc).unwrap().unwrap();
    assert_eq!(recuperada.nome, "Planejamento");
    let voz_alvo = &recuperada
        .trechos
        .iter()
        .find(|t| t.id == alvo)
        .unwrap()
        .voz;
    assert_eq!(
        recuperada
            .vozes
            .iter()
            .find(|v| &v.id == voz_alvo)
            .unwrap()
            .nome,
        "Bruno"
    );
    assert_eq!(
        amb.palavras(&doc),
        rot.iter().map(|p| p.texto.clone()).collect::<Vec<_>>()
    );
}

// MARK: Teto de saída e estados

#[test]
fn saida_com_escrita_lenta_responde_em_dois_segundos() {
    for custo in [1.2_f64, 3.0_f64] {
        let ligada = Arc::new(AtomicBool::new(false));
        let l2 = ligada.clone();
        let amb = Ambiente::com_escrita(Some(Arc::new(move |dados, destino| {
            if l2.load(Ordering::Acquire) {
                thread::sleep(Duration::from_secs_f64(custo));
            }
            DiscoDeBlocos::escrita_duravel(dados, destino)
        })));
        let doc = amb.nova_reuniao("mac");
        let mut coletor = ColetorDeReuniao::new(doc.clone(), amb.reabrir(), amb.disco.clone(), 0.);
        coletor.abrir(FonteDeAudio::Mac, 0.).unwrap();
        let pcm = pcm_sintetico(&roteiro_padrao(), 4.);
        amb.capturar(
            &mut coletor,
            FonteDeAudio::Mac,
            &pcm[..2 * TAXA_USIZE + 8000],
            0.,
            None,
        );
        let duraveis = amb.armazem.blocos(&doc).unwrap().len();
        ligada.store(true, Ordering::Release);

        let saida = SaidaComPrazo::default();
        let inicio = Instant::now();
        let resposta = Arc::new(Mutex::new(None::<Duration>));
        let r2 = resposta.clone();
        assert!(saida.iniciar(
            Duration::from_secs(2),
            move || {
                let _ = coletor.encerrar(4.);
            },
            move || {
                *r2.lock().unwrap() = Some(inicio.elapsed());
            },
        ));
        assert!(!saida.iniciar(Duration::from_secs(2), || {}, || {}));
        while resposta.lock().unwrap().is_none() {
            thread::sleep(Duration::from_millis(10));
        }
        let tempo = resposta.lock().unwrap().unwrap();
        assert!(
            tempo < Duration::from_millis(2250),
            "custo {custo}: {tempo:?}"
        );
        assert!(amb.armazem.blocos(&doc).unwrap()[..duraveis]
            .iter()
            .all(|b| amb.disco.existe(b)));
        let registrados = amb.armazem.blocos(&doc).unwrap().len();
        if custo < 2.0 {
            assert!(registrados > duraveis, "custo {custo}");
        } else {
            assert_eq!(registrados, duraveis, "custo {custo}");
        }
        ligada.store(false, Ordering::Release);
        thread::sleep(Duration::from_secs_f64(custo.min(1.5)));
    }
}

#[test]
fn saida_rapida_responde_ao_terminar() {
    let saida = SaidaComPrazo::default();
    let inicio = Instant::now();
    let resposta = Arc::new(Mutex::new(None::<Duration>));
    let r2 = resposta.clone();
    assert!(saida.iniciar(
        Duration::from_secs(2),
        || thread::sleep(Duration::from_millis(20)),
        move || *r2.lock().unwrap() = Some(inicio.elapsed()),
    ));
    while resposta.lock().unwrap().is_none() {
        thread::sleep(Duration::from_millis(5));
    }
    assert!(resposta.lock().unwrap().unwrap() < Duration::from_millis(500));
    assert!(!saida.em_curso());
}

#[test]
fn saida_durante_gravacao_pausa_e_passada_final_reabre_recuperavel() {
    let amb = Ambiente::new();
    let rot = roteiro_padrao();
    let gravando = capturada(&amb, 3., FonteDeAudio::Mac, &rot);

    let pausada = amb.nova_reuniao("mac");
    let mut c2 = ColetorDeReuniao::new(pausada.clone(), amb.reabrir(), amb.disco.clone(), 0.);
    c2.abrir(FonteDeAudio::Mac, 0.).unwrap();
    amb.capturar(
        &mut c2,
        FonteDeAudio::Mac,
        &pcm_sintetico(&rot, 2.5),
        0.,
        None,
    );
    c2.pausar(2.5).unwrap();
    amb.armazem
        .mudar_estado(&pausada, "pausada", &["gravando"])
        .unwrap();

    let final_doc = capturada(&amb, 5., FonteDeAudio::Mac, &rot);
    let oraculo = Oraculo::new(rot);
    {
        let mut arm = amb.reabrir();
        ProcessadorDeReuniao::new(&final_doc, &mut arm, &amb.disco, &oraculo)
            .fechar()
            .unwrap();
    }
    amb.armazem
        .mudar_estado(&final_doc, "processando", &["gravando"])
        .unwrap();

    let mut reaberto = amb.reabrir();
    reaberto.preparar_abertura(&amb.disco);
    let lista: Vec<String> = reaberto
        .recuperaveis(None)
        .unwrap()
        .into_iter()
        .map(|d| d.id)
        .collect();
    assert_eq!(
        lista.iter().cloned().collect::<HashSet<_>>(),
        HashSet::from([gravando, pausada.clone(), final_doc])
    );
    let total_pausada: usize = reaberto
        .blocos(&pausada)
        .unwrap()
        .iter()
        .map(|b| b.quantidade)
        .sum();
    assert_eq!(total_pausada, (2.5 * f64::from(TAXA)) as usize);
    let id = Identificador {
        faixas: vec![FaixaDeVoz {
            voz: "a".into(),
            inicio: 0.,
            fim: 5.,
        }],
        falhar: false,
    };
    for doc in lista {
        let mut p = ProcessadorDeReuniao::new(&doc, &mut reaberto, &amb.disco, &oraculo);
        p.recuperar().unwrap();
        assert!(p.passada_final(Some(&id)).unwrap());
    }
    assert!(reaberto.recuperaveis(None).unwrap().is_empty());
}

// MARK: Garantia 7: versões de arquivo

fn publicar(amb: &mut Ambiente, v: &Versao, texto: &str) -> bool {
    let trechos = if texto.is_empty() {
        vec![]
    } else {
        vec![TrechoNovo {
            id: format!("{}|0", v.id),
            fonte: FonteDeAudio::Arquivo,
            inicio: 0.,
            fim: 1.,
            texto: texto.into(),
            voz: "a".into(),
        }]
    };
    amb.armazem
        .publicar(&Publicacao {
            versao: v.id.clone(),
            identidade: format!("id-{texto}"),
            trechos,
            duracao: 1.,
            bruto: texto.into(),
            processamento: 0.1,
            baixa_confianca: false,
            quantidade_pcm: None,
        })
        .unwrap()
}

#[test]
fn versao_cancelada_antes_da_publicacao_preserva_a_anterior() {
    let mut amb = Ambiente::new();
    let doc = novo_uuid();
    let v1 = amb
        .armazem
        .criar_versao(&doc, "a.wav", "/tmp/a.wav")
        .unwrap();
    assert!(publicar(&mut amb, &v1, "primeira"));
    let v2 = amb
        .armazem
        .criar_versao(&doc, "a.wav", "/tmp/a.wav")
        .unwrap();
    assert!(amb
        .armazem
        .encerrar_versao(&v2.id, "cancelada", None)
        .unwrap());
    assert!(!publicar(&mut amb, &v2, "cancelada"));
    assert_eq!(amb.textos(&doc), vec!["primeira"]);
    let v3 = amb
        .armazem
        .criar_versao(&doc, "a.wav", "/tmp/a.wav")
        .unwrap();
    assert!(!publicar(&mut amb, &v3, ""));
    assert_eq!(amb.textos(&doc), vec!["primeira"]);
}

#[test]
fn conclusao_fora_de_ordem_nao_torna_visivel_a_tentativa_antiga() {
    let mut amb = Ambiente::new();
    let doc = novo_uuid();
    let antiga = amb
        .armazem
        .criar_versao(&doc, "a.wav", "/tmp/a.wav")
        .unwrap();
    let nova = amb
        .armazem
        .criar_versao(&doc, "a.wav", "/tmp/a.wav")
        .unwrap();
    assert!(publicar(&mut amb, &nova, "nova"));
    assert!(!publicar(&mut amb, &antiga, "antiga"));
    assert_eq!(amb.textos(&doc), vec!["nova"]);
    assert_eq!(
        amb.armazem
            .versoes(&doc)
            .unwrap()
            .into_iter()
            .find(|v| v.id == antiga.id)
            .unwrap()
            .estado,
        "superada"
    );
}

#[test]
fn arquivo_que_muda_durante_a_tentativa_nao_publica() {
    let mut amb = Ambiente::new();
    let doc = novo_uuid();
    let rot = roteiro(&["arquivo", "sintético"], 0.5, 0.35, 0.4);
    let pcm = pcm_sintetico(&rot, 3.);
    let oraculo = Oraculo::new(rot);
    let v1 = amb
        .armazem
        .criar_versao(&doc, "a.wav", "/tmp/a.wav")
        .unwrap();
    {
        let mut arm = amb.reabrir();
        let mut proc = ProcessadorDeArquivo {
            armazem: &mut arm,
            reconhecedor: &oraculo,
            identificador: None,
        };
        let desfecho = proc
            .transcrever(&v1, &pcm, None, |s| s.into(), || Ok(()), |_, _| {})
            .unwrap();
        assert_eq!(desfecho, Desfecho::Publicada);
    }
    assert_eq!(amb.armazem.documento(&doc).unwrap().unwrap().fase, "pronto");
    let v2 = amb
        .armazem
        .criar_versao(&doc, "a.wav", "/tmp/a.wav")
        .unwrap();
    {
        let mut arm = amb.reabrir();
        let mut proc = ProcessadorDeArquivo {
            armazem: &mut arm,
            reconhecedor: &oraculo,
            identificador: None,
        };
        let err = proc.transcrever(
            &v2,
            &pcm,
            None,
            |s| s.into(),
            || Err("mudou".into()),
            |_, _| {},
        );
        assert!(err.is_err());
    }
    assert_eq!(
        amb.armazem.versao_ativa(&doc).unwrap().as_deref(),
        Some(v1.id.as_str())
    );
    assert_eq!(amb.palavras(&doc), vec!["arquivo", "sintético"]);
}

#[test]
fn ditado_comum_intacto_com_armazem_aberto() {
    let amb = Ambiente::new();
    let banco = Banco::new(&amb.banco).unwrap();
    let id = banco
        .inserir(
            &Ditado {
                iniciado_em: 2.,
                estado: "gravando".into(),
                ..Default::default()
            },
            None,
        )
        .unwrap();
    banco.gravar_trecho(id, 0, "trecho em andamento").unwrap();
    let _ = amb.nova_reuniao("mac");
    assert_eq!(banco.trechos(id).unwrap(), vec!["trecho em andamento"]);
    let id2 = banco
        .inserir(
            &Ditado {
                iniciado_em: 1.,
                palavras: 3,
                texto_bruto: "ditado comum".into(),
                texto: "Ditado comum.".into(),
                estado: "concluido".into(),
                ..Default::default()
            },
            None,
        )
        .unwrap();
    assert!(id2 > 0);
    assert_eq!(banco.historico(50, None, "").unwrap().len(), 1);
}

// MARK: Costura, zero trechos, linha do tempo e atalho (CosturaETempoTests)

#[test]
fn palavra_que_cruza_o_corte_entre_blocos_sai_uma_vez_e_inteira() {
    let textos: Vec<String> = (1..=24).map(|n| format!("p{n}")).collect();
    let refs: Vec<&str> = textos.iter().map(String::as_str).collect();
    let rot = roteiro(&refs, 0.85, 0.45, 0.3);
    let oraculo = Oraculo::new(rot.clone());
    let (amb, doc) = ao_vivo(&rot, 20., &oraculo);
    assert_eq!(amb.palavras(&doc), textos);
    assert!(!amb.textos(&doc).join(" ").contains('~'));
}

#[test]
fn duas_repeticoes_reais_proximas_sao_mantidas() {
    let rot = vec![
        PalavraReconhecida {
            texto: "sim".into(),
            inicio: 1.0,
            fim: 1.3,
        },
        PalavraReconhecida {
            texto: "sim".into(),
            inicio: 1.42,
            fim: 1.72,
        },
        PalavraReconhecida {
            texto: "claro".into(),
            inicio: 3.0,
            fim: 3.4,
        },
    ];
    let oraculo = Oraculo::new(rot.clone());
    let (amb, doc) = ao_vivo(&rot, 6., &oraculo);
    assert_eq!(amb.palavras(&doc), vec!["sim", "sim", "claro"]);
}

#[test]
fn fala_continua_sem_pausa_nao_perde_nem_duplica() {
    let textos: Vec<String> = (1..=60).map(|n| format!("c{n}")).collect();
    let refs: Vec<&str> = textos.iter().map(String::as_str).collect();
    let rot = roteiro(&refs, 0.3, 0.4, 0.05);
    let oraculo = Oraculo::new(rot.clone());
    let (amb, doc) = ao_vivo(&rot, 29., &oraculo);
    assert_eq!(amb.palavras(&doc), textos);
}

#[test]
fn ultima_palavra_sem_silencio_entra_no_fechamento() {
    let rot = roteiro(&["combinado", "até", "quinta"], 1.0, 0.5, 0.2);
    let fim = rot.last().unwrap().fim;
    let oraculo = Oraculo::new(rot.clone());
    let (amb, doc) = ao_vivo(&rot, fim, &oraculo);
    assert_eq!(amb.palavras(&doc), vec!["combinado", "até", "quinta"]);
}

#[test]
fn vazio_com_voz_fica_pendente_e_nova_tentativa_reconhece() {
    let rot = roteiro(&["isto", "precisa", "aparecer"], 1.0, 0.35, 0.4);
    let teimoso = Oraculo::com_vazias(rot.clone(), 1000);
    let (amb, doc) = ao_vivo(&rot, 5., &teimoso);
    assert!(amb.textos(&doc).is_empty());
    assert_eq!(
        amb.armazem
            .pendencias(&doc)
            .unwrap()
            .iter()
            .filter(|p| p.estado == "aberta")
            .count(),
        1
    );
    assert!(amb
        .armazem
        .blocos(&doc)
        .unwrap()
        .iter()
        .any(|b| b.estado == EstadoBloco::Pendente));
    assert!(amb
        .armazem
        .documento(&doc)
        .unwrap()
        .unwrap()
        .pendentes
        .is_some());

    let bom = Oraculo::new(rot);
    let mut r = amb.reabrir();
    ProcessadorDeReuniao::new(&doc, &mut r, &amb.disco, &bom)
        .recuperar()
        .unwrap();
    assert_eq!(amb.palavras(&doc), vec!["isto", "precisa", "aparecer"]);
    assert!(!amb
        .armazem
        .blocos(&doc)
        .unwrap()
        .iter()
        .any(|b| matches!(b.estado, EstadoBloco::Pendente | EstadoBloco::Capturado)));
    ProcessadorDeReuniao::new(&doc, &mut r, &amb.disco, &bom)
        .recuperar()
        .unwrap();
    assert_eq!(amb.palavras(&doc), vec!["isto", "precisa", "aparecer"]);
}

#[test]
fn vazio_persistente_continua_repetivel_com_contador() {
    let rot = roteiro(&["fala"], 1.0, 0.35, 0.4);
    let o1 = Oraculo::com_vazias(rot.clone(), 1000);
    let (amb, doc) = ao_vivo(&rot, 3., &o1);
    let o2 = Oraculo::com_vazias(rot, 1000);
    let mut arm = amb.reabrir();
    ProcessadorDeReuniao::new(&doc, &mut arm, &amb.disco, &o2)
        .recuperar()
        .unwrap();
    let p = amb
        .armazem
        .pendencias(&doc)
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    assert_eq!(p.estado, "aberta");
    assert_eq!(p.tentativas, 2);
    assert!(p.motivo.contains("2 tentativas"));
}

#[test]
fn silencio_confirmado_e_terminal() {
    let oraculo = Oraculo::new(vec![]);
    let (amb, doc) = ao_vivo(&[], 4., &oraculo);
    let blocos = amb.armazem.blocos(&doc).unwrap();
    assert!(!blocos.is_empty());
    assert!(blocos.iter().all(|b| b.estado == EstadoBloco::SemFala));
    assert!(amb.armazem.pendencias(&doc).unwrap().is_empty());
    let mut arm = amb.reabrir();
    ProcessadorDeReuniao::new(&doc, &mut arm, &amb.disco, &oraculo)
        .recuperar()
        .unwrap();
    assert_eq!(oraculo.chamadas(), 0);
}

#[test]
fn criterio_de_atividade_com_fala_curta_e_baixa() {
    let mut curta = vec![0f32; 16000];
    for (i, s) in curta.iter_mut().enumerate().take(10400).skip(8000) {
        *s = 0.02 * ((i as f64) * 0.3).sin() as f32;
    }
    assert!(AtividadeDeVoz::tem(&curta));
    let chiado: Vec<f32> = (0u32..16000)
        .map(|i| {
            ((i.wrapping_mul(1103515245).wrapping_add(12345) % 1000) as f32 / 1000. - 0.5) * 0.005
        })
        .collect();
    assert!(!AtividadeDeVoz::tem(&chiado));
    let mut clique = vec![0f32; 16000];
    for s in clique.iter_mut().take(400).skip(100) {
        *s = 0.5;
    }
    assert!(!AtividadeDeVoz::tem(&clique));
}

#[test]
fn linha_do_tempo_microfone_deriva_e_pausa() {
    let linha = LinhaDoTempo::new(100.);
    let mut mac = CursorDeFonte::new(0);
    let mut mic = CursorDeFonte::new(0);
    assert_eq!(
        mac.receber(100.0, 1600, &linha),
        vec![Evento::Amostras(Parte {
            origem: 0,
            quantidade: 1600,
            indice: 0
        })]
    );
    assert_eq!(
        mic.receber(100.0, 1600, &linha),
        vec![Evento::Amostras(Parte {
            origem: 0,
            quantidade: 1600,
            indice: 0
        })]
    );
    assert_eq!(
        mic.receber(100.1, 1600, &linha),
        vec![Evento::Amostras(Parte {
            origem: 0,
            quantidade: 1600,
            indice: 1600
        })]
    );

    let linha2 = LinhaDoTempo::new(0.);
    let mut c = CursorDeFonte::new(0);
    let _ = c.receber(0., 1600, &linha2);
    assert_eq!(
        c.receber(0.13, 1600, &linha2),
        vec![Evento::Amostras(Parte {
            origem: 0,
            quantidade: 1600,
            indice: 1600
        })]
    );
    assert_eq!(
        c.receber(0.5, 1600, &linha2),
        vec![
            Evento::Lacuna(3200..8000),
            Evento::Amostras(Parte {
                origem: 0,
                quantidade: 1600,
                indice: 8000
            })
        ]
    );
    assert!(c.receber(0.4, 3200, &linha2).is_empty());
    assert_eq!(c.proximo, 9600);

    let mut linha3 = LinhaDoTempo::new(0.);
    let mut c3 = CursorDeFonte::new(15200);
    linha3.pausar(1.0);
    assert_eq!(
        c3.receber(0.95, 1600, &linha3),
        vec![Evento::Amostras(Parte {
            origem: 0,
            quantidade: 800,
            indice: 15200
        })]
    );
    assert!(c3.receber(2.0, 1600, &linha3).is_empty());
    linha3.retomar(5.0);
    assert_eq!(linha3.indice(5.0), Some(16000));
    let mut depois = CursorDeFonte::new(16000);
    assert_eq!(
        depois.receber(5.0, 1600, &linha3),
        vec![Evento::Amostras(Parte {
            origem: 0,
            quantidade: 1600,
            indice: 16000
        })]
    );
}

#[test]
fn troca_de_dispositivo_abre_nova_geracao_com_lacuna_visivel() {
    let amb = Ambiente::new();
    let doc = amb.nova_reuniao("macEMicrofone");
    let mut coletor = ColetorDeReuniao::new(doc.clone(), amb.reabrir(), amb.disco.clone(), 0.);
    let antes = roteiro(&["antes", "da", "troca"], 0.2, 0.3, 0.25);
    let depois = roteiro(&["depois", "da", "troca"], 3.2, 0.3, 0.25);
    coletor.abrir(FonteDeAudio::Mic, 0.).unwrap();
    amb.capturar(
        &mut coletor,
        FonteDeAudio::Mic,
        &pcm_sintetico(&antes, 2.5),
        0.,
        None,
    );
    let nova_geracao = coletor.abrir(FonteDeAudio::Mic, 3.0).unwrap();
    assert_eq!(nova_geracao, 1);
    let pcm = pcm_sintetico(&depois, 6.);
    amb.capturar(
        &mut coletor,
        FonteDeAudio::Mic,
        &pcm[3 * TAXA_USIZE..],
        3.0,
        None,
    );
    coletor.encerrar(6.).unwrap();
    let mut tudo = antes;
    tudo.extend(depois);
    let oraculo = Oraculo::new(tudo);
    {
        let mut arm = amb.reabrir();
        ProcessadorDeReuniao::new(&doc, &mut arm, &amb.disco, &oraculo)
            .fechar()
            .unwrap();
    }
    let blocos = amb.armazem.blocos(&doc).unwrap();
    assert_eq!(
        blocos.iter().map(|b| b.id.geracao).collect::<HashSet<_>>(),
        HashSet::from([0, 1])
    );
    assert_eq!(amb.armazem.costuras(&doc).unwrap().len(), 2);
    let doc2 = amb.armazem.documento(&doc).unwrap().unwrap();
    let lacuna = &doc2.lacunas.as_ref().unwrap()[0];
    assert!((lacuna.inicio - 2.5).abs() < 0.01);
    assert!((lacuna.fim - 3.0).abs() < 0.01);
    assert_eq!(
        amb.palavras(&doc),
        vec!["antes", "da", "troca", "depois", "da", "troca"]
    );
    assert!(doc2
        .trechos
        .iter()
        .all(|t| t.voz == "voce" && t.origem.as_deref() == Some("microfone")));
    assert!((doc2.trechos.last().unwrap().inicio - 3.2).abs() < 0.01);
}

#[test]
fn atalho_durante_reuniao_usa_derivacao_propria() {
    let reuniao = Arc::new(Mutex::new(Vec::<f32>::new()));
    let r2 = reuniao.clone();
    let mut dist = DistribuidorDoMicrofone::new(move |_, amostras: &[f32]| {
        r2.lock().unwrap().extend_from_slice(amostras)
    });
    dist.entregar(0., &[1., 1.]);
    let ditado = dist.abrir_derivacao(16);
    dist.entregar(0.1, &[2., 2.]);
    dist.fechar_derivacao();
    dist.entregar(0.2, &[3., 3.]);
    let recebidos: Vec<f32> = ditado.into_iter().flatten().collect();
    assert_eq!(recebidos, vec![2., 2.]);
    assert_eq!(*reuniao.lock().unwrap(), vec![1., 1., 2., 2., 3., 3.]);

    let _lenta = dist.abrir_derivacao(1);
    for i in 0..5 {
        dist.entregar(1. + i as f64, &[9.]);
    }
    assert!(!dist.derivacao_aberta());
    assert_eq!(dist.estouros_da_derivacao, 1);
    assert_eq!(reuniao.lock().unwrap().len(), 11);
}

#[test]
fn cauda_no_fechamento_expande_contexto_quando_janela_curta_falha() {
    struct Exigente(Vec<PalavraReconhecida>);
    impl ReconhecedorDeFala for Exigente {
        fn palavras(
            &self,
            amostras: &[f32],
            inicio: f64,
            _fim: Option<f64>,
        ) -> Result<Vec<PalavraReconhecida>, Erro> {
            let fim = inicio + amostras.len() as f64 / f64::from(TAXA);
            Ok(self
                .0
                .iter()
                .filter(|p| {
                    let c = (p.inicio + p.fim) / 2.;
                    if p.texto == "combinado" {
                        amostras.len() > 4 * TAXA_USIZE && c >= inicio && c <= fim
                    } else {
                        c >= inicio && c <= fim
                    }
                })
                .cloned()
                .collect())
        }
    }
    let mut rot = roteiro(&["primeira", "frase", "da", "reunião"], 0.5, 0.4, 0.3);
    rot.push(PalavraReconhecida {
        texto: "combinado".into(),
        inicio: 6.2,
        fim: 6.8,
    });
    let amb = Ambiente::new();
    let doc = amb.nova_reuniao("mac");
    let mut coletor = ColetorDeReuniao::new(doc.clone(), amb.reabrir(), amb.disco.clone(), 0.);
    let exigente = Exigente(rot.clone());
    coletor.abrir(FonteDeAudio::Mac, 0.).unwrap();
    let pcm = pcm_sintetico(&rot, 7.);
    amb.capturar(
        &mut coletor,
        FonteDeAudio::Mac,
        &pcm[..5 * TAXA_USIZE],
        0.,
        None,
    );
    {
        let mut arm = amb.reabrir();
        ProcessadorDeReuniao::new(&doc, &mut arm, &amb.disco, &exigente)
            .fechar()
            .unwrap();
    }
    amb.capturar(
        &mut coletor,
        FonteDeAudio::Mac,
        &pcm[5 * TAXA_USIZE..],
        5.0,
        None,
    );
    coletor.encerrar(7.).unwrap();
    let c = rusqlite::Connection::open(&amb.banco).unwrap();
    c.execute(
        "UPDATE lt_costuras SET fechada = 0 WHERE documento = ?",
        [&doc],
    )
    .unwrap();
    {
        let mut arm = amb.reabrir();
        ProcessadorDeReuniao::new(&doc, &mut arm, &amb.disco, &exigente)
            .fechar()
            .unwrap();
    }
    assert_eq!(
        amb.palavras(&doc),
        vec!["primeira", "frase", "da", "reunião", "combinado"]
    );
    assert!(amb.armazem.pendencias(&doc).unwrap().is_empty());
}

#[test]
fn lista_leve_traz_contagem_de_palavras_e_previa() {
    let tmp = tempfile::tempdir().unwrap();
    let banco = Banco::new(tmp.path().join("teste.sqlite")).unwrap();
    let textos = ["Vamos revisar o plano.", "Combinado, até quinta-feira."];
    let arquivo = TranscricaoLonga {
        id: novo_uuid(),
        nome: "SINTETICO.WAV".into(),
        fase: "pronto".into(),
        criado_em: 100.,
        trechos: textos
            .iter()
            .enumerate()
            .map(|(i, t)| TrechoLongo {
                id: format!("t{i}"),
                inicio: i as f64,
                fim: i as f64 + 1.,
                voz: "a".into(),
                texto: (*t).into(),
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    };
    banco.salvar_transcricao(&arquivo).unwrap();
    let vazia = TranscricaoLonga {
        id: novo_uuid(),
        nome: "Silêncio".into(),
        fase: "pronta".into(),
        fonte: Some("mac".into()),
        criado_em: 90.,
        ..Default::default()
    };
    banco.salvar_transcricao(&vazia).unwrap();

    let leve = banco
        .transcricoes("arquivo", false)
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    assert!(leve.trechos.is_empty());
    assert_eq!(
        leve.palavras,
        Some(textos.iter().map(|t| Palavras::contar(t)).sum())
    );
    assert_eq!(leve.previa.as_deref(), Some(textos[0]));
    let sem_fala = banco
        .transcricoes("reuniao", false)
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    assert_eq!(sem_fala.palavras, Some(0));
    assert!(sem_fala.previa.is_none());
    assert!(banco
        .transcricao(&arquivo.id)
        .unwrap()
        .unwrap()
        .palavras
        .is_none());
}
