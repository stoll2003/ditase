//! Testes de tempo da saída com prazo, num binário próprio.
//!
//! O `cargo test` roda os testes de um binário em paralelo, mas um binário de cada vez.
//! Em `persistencia.rs` este teste dividia a máquina com outros 33, todos gravando PCM com
//! `sync_all` e SQLite com `synchronous=FULL` no mesmo disco. O cenário de 1,2 s deixa 0,8 s
//! até o prazo para a E/S real (temporário, FlushFileBuffers, renomeação com write-through e
//! commit do WAL). No runner Windows do GitHub essa folga foi consumida pela disputa de disco
//! com os outros testes; aqui o teste mede o tempo sozinho, e a mensagem de falha traz
//! quanto levou cada etapa.
mod comum;

use nucleo::{motores::TAXA, *};
use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

const TAXA_USIZE: usize = TAXA as usize;

fn novo_uuid() -> String {
    static N: AtomicU64 = AtomicU64::new(1);
    let n = N.fetch_add(1, Ordering::Relaxed);
    format!("{n:08x}-0000-4000-8000-00000000abcd")
}

/// Fala sintética: cinco palavras de 0,4 s com pausas de 0,35 s a partir de 0,4 s.
fn pcm_sintetico(segundos: f64) -> Vec<f32> {
    let mut saida = vec![0f32; (segundos * f64::from(TAXA)) as usize];
    for k in 0..5 {
        let a = 0.4 + 0.75 * f64::from(k);
        let (x, z) = (
            (a * f64::from(TAXA)) as usize,
            saida.len().min(((a + 0.4) * f64::from(TAXA)) as usize),
        );
        for (i, s) in saida.iter_mut().enumerate().take(z).skip(x) {
            *s = 0.1 * ((i as f64) * 0.3).sin() as f32;
        }
    }
    saida
}

#[derive(Default)]
struct Marcas {
    /// Tempo real de `escrita_duravel`, sem o atraso simulado.
    io: Option<Duration>,
    fim_escrita: Option<Instant>,
    fim_trabalho: Option<Instant>,
}

#[test]
fn saida_com_escrita_lenta_responde_em_dois_segundos() {
    for custo in [1.2_f64, 3.0_f64] {
        let tmp = comum::tempdir();
        let banco = tmp.path().join("teste.sqlite");
        let armazem = ArmazemLancamento::new(&banco).unwrap();
        let ligada = Arc::new(AtomicBool::new(false));
        let marcas = Arc::new(Mutex::new(Marcas::default()));
        let (l2, m2) = (ligada.clone(), marcas.clone());
        let disco = DiscoDeBlocos::com_escrita(
            tmp.path().join("Audio"),
            Arc::new(move |dados, destino| {
                if !l2.load(Ordering::Acquire) {
                    return DiscoDeBlocos::escrita_duravel(dados, destino);
                }
                thread::sleep(Duration::from_secs_f64(custo));
                let t = Instant::now();
                let r = DiscoDeBlocos::escrita_duravel(dados, destino);
                let mut m = m2.lock().unwrap();
                m.io = Some(t.elapsed());
                m.fim_escrita = Some(Instant::now());
                r
            }),
        );
        let doc = novo_uuid();
        armazem
            .criar_documento(
                &doc,
                "reuniao",
                "Reunião sintética",
                Some("mac"),
                "gravando",
            )
            .unwrap();
        let mut coletor = ColetorDeReuniao::new(
            doc.clone(),
            ArmazemLancamento::new(&banco).unwrap(),
            disco.clone(),
            0.,
        );
        coletor.abrir(FonteDeAudio::Mac, 0.).unwrap();
        // 2,5 s: dois blocos de 1 s selados e duráveis, meio segundo ainda em memória.
        let pcm = pcm_sintetico(4.);
        let passo = TAXA_USIZE / 10;
        let fim = 2 * TAXA_USIZE + 8000;
        let mut i = 0;
        while i < fim {
            let z = fim.min(i + passo);
            coletor
                .receber(FonteDeAudio::Mac, i as f64 / f64::from(TAXA), &pcm[i..z])
                .unwrap();
            i += passo;
        }
        let duraveis = armazem.blocos(&doc).unwrap().len();
        assert_eq!(duraveis, 2);
        ligada.store(true, Ordering::Release);

        let saida = SaidaComPrazo::default();
        let inicio = Instant::now();
        let resposta = Arc::new(Mutex::new(None::<Duration>));
        let (r2, m3) = (resposta.clone(), marcas.clone());
        assert!(saida.iniciar(
            Duration::from_secs(2),
            move || {
                let _ = coletor.encerrar(4.);
                m3.lock().unwrap().fim_trabalho = Some(Instant::now());
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
        let na_resposta = armazem.blocos(&doc).unwrap();
        // Estado no instante da resposta; as conferências vêm depois de o trabalho terminar,
        // para a mensagem de falha trazer o tempo de cada etapa.
        let registrados = na_resposta.len();
        let todos_duraveis = na_resposta.iter().all(|b| disco.existe(b));
        let espera = Instant::now();
        while marcas.lock().unwrap().fim_trabalho.is_none()
            && espera.elapsed() < Duration::from_secs(15)
        {
            thread::sleep(Duration::from_millis(20));
        }
        let m = marcas.lock().unwrap();
        let rel = |t: Option<Instant>| t.map(|t| t.duration_since(inicio));
        let medida = format!(
            "custo {custo}: resposta {tempo:?}, registrados na resposta {registrados} (antes {duraveis}), \
             E/S real {:?}, escrita terminou em {:?}, registro terminou em {:?}",
            m.io,
            rel(m.fim_escrita),
            rel(m.fim_trabalho)
        );
        assert!(tempo < Duration::from_millis(2250), "{medida}");
        // Nada registrado sem estar durável.
        assert!(todos_duraveis, "{medida}");
        if custo < 2.0 {
            // Escrita que termina antes do prazo é registrada antes da resposta.
            assert!(registrados > duraveis, "{medida}");
        } else {
            // A escrita de 3 s ainda não acabou: o bloco não pode estar registrado.
            assert_eq!(registrados, duraveis, "{medida}");
        }
        // Depois da resposta o trabalho segue até o fim e registra o bloco escrito.
        assert!(m.fim_trabalho.is_some(), "{medida}");
        let ao_fim = armazem.blocos(&doc).unwrap();
        assert_eq!(ao_fim.len(), duraveis + 1, "{medida}");
        assert!(ao_fim.iter().all(|b| disco.existe(b)), "{medida}");
        drop(m);
        ligada.store(false, Ordering::Release);
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
