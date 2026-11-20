use nucleo::*;
use serde::Deserialize;
use std::collections::BTreeMap;
#[derive(Deserialize)]
struct Texto {
    operacao: String,
    entrada: String,
    esperado: String,
}
#[test]
fn regras_de_fala() {
    let casos: BTreeMap<String, Vec<Texto>> =
        serde_json::from_str(include_str!("../../contrato/casos/regras_fala.json")).unwrap();
    for (nome, itens) in casos {
        for c in itens {
            let r = match c.operacao.as_str() {
                "filtrar" => RegrasFala::filtrar(&c.entrada),
                "arrumarEspacos" => RegrasFala::arrumar_espacos(&c.entrada),
                _ => panic!("operação desconhecida"),
            };
            assert_eq!(r, c.esperado, "{nome}")
        }
    }
}
#[derive(Deserialize)]
struct CasoFidelidade {
    original: String,
    modelo: String,
    esperado: Option<String>,
}
#[test]
fn fidelidade() {
    let casos: BTreeMap<String, Vec<CasoFidelidade>> =
        serde_json::from_str(include_str!("../../contrato/casos/fidelidade.json")).unwrap();
    for (nome, itens) in casos {
        for c in itens {
            let r = Fidelidade::fundir(&c.original, &c.modelo);
            if let Some(e) = c.esperado {
                assert_eq!(r, e, "{nome}")
            } else if nome == "testRestauraFraseCortada" {
                assert!(
                    r.contains("eu quero montar um filtro aqui que organiza isso")
                        && r.ends_with("Depois a gente vê.")
                )
            } else {
                assert_eq!(nome, "testCorrecaoNaoAtravessaFraseEncerrada");
                assert!(
                    r.starts_with("O ensaio começa na sexta.")
                        && r.to_lowercase().ends_with("reserva a sala verde.")
                        && !r.contains("azul")
                )
            }
        }
    }
}
#[derive(Deserialize)]
struct Parte {
    fala: bool,
    segundos: f64,
}
#[test]
fn segmentacao() {
    let casos: BTreeMap<String, Vec<Parte>> =
        serde_json::from_str(include_str!("../../contrato/casos/segmentador.json")).unwrap();
    for (nome, partes) in casos {
        let mut s = Segmentador::default();
        let mut pcm = vec![];
        let mut fase = 0.;
        for p in partes {
            for _ in 0..(p.segundos * 16000.) as usize {
                fase += 2. * std::f64::consts::PI * 220. / 16000.;
                pcm.push(if p.fala { fase.sin() as f32 * 0.2 } else { 0. })
            }
        }
        if nome.contains("VozBaixa") {
            for a in &mut pcm {
                *a *= 0.05
            }
        }
        let mut ts = vec![];
        if nome.contains("UmaHora") {
            for _ in 0..410 {
                ts.extend(s.alimentar(&pcm).into_iter().map(|mut s| {
                    s.amostras.clear();
                    s
                }))
            }
        } else {
            for q in pcm.chunks(1024) {
                ts.extend(s.alimentar(q))
            }
        }
        ts.extend(s.finalizar());
        match nome.as_str() {
            "testSilencioNaoGeraTrecho" | "testCliqueCurtoEhDescartado" => assert!(ts.is_empty()),
            "testFalaCurtaViraUmTrechoComMargem" => {
                assert_eq!(ts.len(), 1);
                assert!(
                    (ts[0].duracao() - 2.4).abs() < 0.25
                        && (ts[0].pausa_antes - 1.).abs() < 0.1
                        && !ts[0].cortado_no_meio
                )
            }
            "testPausaCurtaNaoCortaTrechoPequeno" | "testVozBaixaAindaEhFala" => {
                assert_eq!(ts.len(), 1)
            }
            "testPausaCortaDepoisDoMinimo" => {
                assert_eq!(ts.len(), 2);
                assert!((ts[0].duracao() - 6.2).abs() < 0.3)
            }
            "testPausaLongaCortaSempreEInformaAPausa" => {
                assert_eq!(ts.len(), 2);
                assert!((ts[1].pausa_antes - 3.).abs() < 0.15)
            }
            "testFalaSemPausaCortaNoLimiteSemPerderAmostra" => {
                assert!(ts.len() >= 3 && ts.iter().all(|s| s.duracao() <= 13.6));
                assert_eq!(
                    ts.iter().map(|s| s.amostras.len()).sum::<usize>(),
                    pcm.len()
                );
                for par in ts.windows(2) {
                    assert!(par[0].cortado_no_meio);
                    assert_eq!(par[0].inicio + par[0].amostras.len(), par[1].inicio)
                }
            }
            "testCorteForcadoProcuraOPontoQuieto" => {
                assert_eq!(ts.len(), 2);
                assert!((ts[0].duracao() - 11.15).abs() < 0.3)
            }
            "testUmaHoraDeFalaNaoAcumulaMemoriaNemPerdeTrecho" => {
                assert_eq!(ts.len(), 410);
                assert!((s.fala_total - 3280.).abs() < 60.)
            }
            _ => panic!("caso desconhecido {nome}"),
        }
    }
}

#[test]
fn fronteira() {
    #[derive(Deserialize)]
    struct P {
        texto: String,
        inicio: f64,
        fim: f64,
    }
    #[derive(Deserialize)]
    struct Caso {
        palavras: Vec<P>,
        confirmado: usize,
        limite: usize,
        fechamento: bool,
        atividade: bool,
        textos: Vec<String>,
        acima: usize,
        ate: usize,
    }
    let c: Caso = serde_json::from_str(include_str!("../../contrato/casos/costura.json")).unwrap();
    let ps = c
        .palavras
        .into_iter()
        .map(|p| motores::PalavraReconhecida {
            texto: p.texto,
            inicio: p.inicio,
            fim: p.fim,
        })
        .collect::<Vec<_>>();
    let r = Costura::decidir(&ps, c.confirmado, c.limite, c.fechamento, |_| c.atividade);
    assert_eq!(
        r.confirmadas
            .into_iter()
            .map(|p| p.texto)
            .collect::<Vec<_>>(),
        c.textos
    );
    assert!(r.novo_confirmado > c.acima && r.novo_confirmado <= c.ate)
}
