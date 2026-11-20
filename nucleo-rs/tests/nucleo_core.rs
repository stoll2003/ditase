use nucleo::{motores::*, *};
use std::sync::{Arc, Mutex};

#[test]
fn banco_ditados_trechos_dicionario_e_config() {
    let tmp = tempfile::tempdir().unwrap();
    let mut banco = Banco::new(tmp.path().join("ditado.sqlite")).unwrap();

    let id = banco
        .inserir(
            &Ditado {
                iniciado_em: 100.,
                estado: "gravando".into(),
                ..Default::default()
            },
            Some("ext-1"),
        )
        .unwrap();
    assert!(banco.existe("ext-1").unwrap());
    assert_eq!(banco.interrompidos().unwrap().len(), 1);
    banco.gravar_trecho(id, 0, "olá").unwrap();
    banco.gravar_trecho(id, 1, "mundo").unwrap();
    assert_eq!(banco.trechos(id).unwrap(), vec!["olá", "mundo"]);

    let mut d = banco.ditado(id).unwrap().unwrap();
    d.estado = "concluido".into();
    d.texto = "Olá mundo.".into();
    d.palavras = 2;
    banco.concluir(&d).unwrap();
    assert!(banco.trechos(id).unwrap().is_empty());
    assert!(banco.interrompidos().unwrap().is_empty());
    assert_eq!(
        banco.ultimo_concluido().unwrap().unwrap().texto,
        "Olá mundo."
    );
    assert_eq!(banco.historico(10, None, "mundo").unwrap().len(), 1);

    let t = banco
        .salvar(Termo {
            id: 0,
            termo: "Dita.se".into(),
            ouvido_como: vec!["dita ce".into(), "dita.se".into()],
            usos: 0,
            origem: "manual".into(),
            criado_em: 1.,
        })
        .unwrap();
    assert_eq!(t.ouvido_como, vec!["dita ce"]);
    banco.contar_uso(&[t.id]).unwrap();
    assert_eq!(banco.termos().unwrap()[0].usos, 1);

    banco.definir("modelo", "qwen").unwrap();
    assert_eq!(banco.config("modelo").unwrap().as_deref(), Some("qwen"));
}

#[test]
fn dicionario_e_guarda_e_alinhamento() {
    let ap = AplicadorDicionario::new(
        vec![Termo {
            id: 7,
            termo: "Dita.se".into(),
            ouvido_como: vec!["dita ce".into()],
            usos: 0,
            origem: "manual".into(),
            criado_em: 0.,
        }],
        None,
    );
    let (res, trocas) = ap.aplicar("eu uso o dita ce todo dia");
    assert_eq!(res, "eu uso o Dita.se todo dia");
    assert_eq!(trocas.len(), 1);
    assert_eq!(trocas[0].termo_id, 7);

    assert_eq!(
        Guarda::avaliar("um dois três quatro", ""),
        Some(Recusa::Vazio)
    );
    assert_eq!(Guarda::avaliar("olá mundo", "Olá, mundo."), None);
    assert_eq!(
        Guarda::avaliar("olá mundo", "Claro! Aqui está o texto: Olá mundo."),
        Some(Recusa::Respondeu)
    );

    let faixas = vec![
        FaixaDeVoz {
            voz: "a".into(),
            inicio: 0.,
            fim: 1.0,
        },
        FaixaDeVoz {
            voz: "b".into(),
            inicio: 1.0,
            fim: 2.0,
        },
    ];
    assert_eq!(AlinhamentoDeVoz::identificar(0.1, 0.9, &faixas), "a");
    assert_eq!(
        AlinhamentoDeVoz::identificar(0.5, 1.5, &faixas),
        "desconhecida"
    );
}

#[test]
fn estatisticas_calcula_totais_e_sequencia() {
    let fuso = chrono::FixedOffset::west_opt(4 * 3600).unwrap();
    let dia = 86400.;
    let agora = 1_700_000_000.;
    let ditados = vec![
        Ditado {
            iniciado_em: agora - dia,
            duracao: 30.,
            duracao_fala: 30.,
            palavras: 60,
            latencia: 0.5,
            app: "app.a".into(),
            app_nome: "App A".into(),
            origem: "ditado".into(),
            ..Default::default()
        },
        Ditado {
            iniciado_em: agora,
            duracao: 60.,
            duracao_fala: 60.,
            palavras: 120,
            latencia: 0.7,
            app: "app.a".into(),
            app_nome: "App A".into(),
            origem: "ditado".into(),
            ..Default::default()
        },
    ];
    let e = Estatisticas::calcular_no_fuso(&ditados, agora, 7, fuso);
    assert_eq!(e.total_palavras, 180);
    assert_eq!(e.total_ditados, 2);
    assert_eq!(e.dias_seguidos, 2);
    assert_eq!(e.maior_sequencia, 2);
    assert_eq!(e.palavras_hoje, 120);
    assert_eq!(e.por_dia.len(), 7);
    assert_eq!(e.por_app[0].nome, "App A");
}

#[test]
fn sessao_ditado_transcreve_aplica_dicionario_e_avisa_trechos() {
    struct Eco;
    impl Transcritor for Eco {
        fn nome(&self) -> &str {
            "eco"
        }
        fn transcrever(&self, _amostras: &[f32]) -> Result<String, Erro> {
            Ok("eu uso dita ce".into())
        }
    }
    let avisos = Arc::new(Mutex::new(Vec::<(usize, String)>::new()));
    let a2 = avisos.clone();
    let dic = AplicadorDicionario::new(
        vec![Termo {
            id: 1,
            termo: "Dita.se".into(),
            ouvido_como: vec!["dita ce".into()],
            ..Default::default()
        }],
        None,
    );
    let mut sessao = SessaoDitado::new(
        Arc::new(Eco),
        None,
        dic,
        ConfigSessao::default(),
        Some(Arc::new(move |ordem, bruto| {
            a2.lock().unwrap().push((ordem, bruto.into()))
        })),
    );
    let mut pcm = Vec::with_capacity(16000);
    for i in 0..16000 {
        pcm.push(0.2 * ((i as f64) * 2. * std::f64::consts::PI * 220. / 16000.).sin() as f32);
    }
    sessao.alimentar(&pcm).unwrap();
    let r = sessao.finalizar().unwrap();
    assert_eq!(r.texto, "Eu uso Dita.se");
    assert_eq!(r.trocas.len(), 1);
    assert_eq!(avisos.lock().unwrap().len(), 1);
}
