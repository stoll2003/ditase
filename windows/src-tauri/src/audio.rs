//! Captura de áudio (porta de `Gravador.swift` e da parte de áudio de `CapturaReuniao.swift`).
//! Microfone e áudio do sistema pelo WASAPI via `cpal`: no Windows, abrir um fluxo de entrada
//! num dispositivo de SAÍDA é o loopback (o que toca no PC), sem permissão especial.
//! Tudo sai em 16 kHz mono f32 (`nucleo::motores::TAXA`), em blocos de cerca de 100 ms.
use std::{
    fs::File,
    io::{self, Seek, SeekFrom, Write},
    path::Path,
    sync::{mpsc, Mutex, OnceLock},
    thread,
};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

pub const TAXA: u32 = nucleo::motores::TAXA;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Fonte { Microfone, Sistema }

pub type Erro = Box<dyn std::error::Error + Send + Sync>;

/// Microfones disponíveis, com o nome servindo de identificador (o WASAPI não expõe o ID pelo cpal).
pub fn microfones() -> Vec<(String, String)> {
    no_fio_de_audio(|| {
        let host = cpal::default_host();
        host.input_devices().map(|lista| lista.filter_map(|d| d.name().ok()).map(|n| (n.clone(), n)).collect()).unwrap_or_default()
    })
}

/// Há microfone utilizável? Sem dispositivo ou com acesso negado em Privacidade, não.
pub fn microfone_disponivel() -> bool { no_fio_de_audio(|| cpal::default_host().default_input_device().is_some()) }

type TarefaDeAudio = Box<dyn FnOnce() + Send>;

/// Todo uso do cpal roda nesta thread, que vive até o processo acabar.
///
/// No Windows o cpal guarda um `IMMDeviceEnumerator` global, criado no apartamento COM da primeira
/// thread que consulta áudio, e cada thread desfaz o próprio COM (`CoUninitialize`) ao terminar.
/// Se a thread que criou o enumerador termina e era a última com COM, o COM do processo é
/// descarregado e o ponteiro global fica pendurado: a consulta seguinte, em outra thread, cai com
/// STATUS_ACCESS_VIOLATION. Aqui o enumerador nasce nesta thread e o COM dela nunca é desfeito.
fn no_fio_de_audio<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    static FIO: OnceLock<Mutex<mpsc::Sender<TarefaDeAudio>>> = OnceLock::new();
    let envio = FIO.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<TarefaDeAudio>();
        thread::Builder::new().name("ditase.com-audio".into()).spawn(move || {
            // Primeira consulta do processo: COM e enumerador ficam presos a esta thread.
            let _ = cpal::default_host().default_input_device();
            for tarefa in rx { tarefa(); }
        }).expect("thread de áudio");
        Mutex::new(tx)
    });
    let (tx, rx) = mpsc::channel();
    let tarefa: TarefaDeAudio = Box::new(move || { let _ = tx.send(f()); });
    envio.lock().unwrap_or_else(|e| e.into_inner()).send(tarefa).expect("thread de áudio encerrada");
    rx.recv().expect("thread de áudio não respondeu")
}

thread_local! {
    /// Fluxos abertos (o `Stream` do cpal não é `Send`): ficam na thread de áudio, por id.
    static FLUXOS: std::cell::RefCell<std::collections::HashMap<u64, cpal::Stream>> = Default::default();
}

/// Passa-baixa Butterworth de 4ª ordem (duas biquads RBJ) seguido de interpolação linear:
/// tira o que passa de 7 kHz antes de reduzir para 16 kHz, para não dobrar em ruído audível.
pub struct Reamostrador { entrada: f64, passo: f64, pos: f64, anterior: f32, filtros: Vec<([f64; 5], [f64; 2])> }

impl Reamostrador {
    pub fn new(taxa_entrada: u32) -> Self {
        let biquad = |q: f64| {
            let w = 2.0 * std::f64::consts::PI * (0.44 * TAXA as f64) / taxa_entrada as f64;
            let alfa = w.sin() / (2.0 * q);
            let c = w.cos();
            let a0 = 1.0 + alfa;
            ([(1.0 - c) / 2.0 / a0, (1.0 - c) / a0, (1.0 - c) / 2.0 / a0, -2.0 * c / a0, (1.0 - alfa) / a0], [0.0; 2])
        };
        let filtros = if taxa_entrada > TAXA { vec![biquad(0.541_196), biquad(1.306_563)] } else { vec![] };
        Self { entrada: taxa_entrada as f64, passo: taxa_entrada as f64 / TAXA as f64, pos: 0.0, anterior: 0.0, filtros }
    }

    /// Recebe mono na taxa de entrada, devolve mono em 16 kHz.
    pub fn processar(&mut self, mono: &[f32]) -> Vec<f32> {
        let mut filtrado = mono.to_vec();
        for ([b0, b1, b2, a1, a2], z) in self.filtros.iter_mut() {
            for x in filtrado.iter_mut() {
                let e = *x as f64;
                let y = *b0 * e + z[0];
                z[0] = *b1 * e - *a1 * y + z[1];
                z[1] = *b2 * e - *a2 * y;
                *x = y as f32;
            }
        }
        if self.entrada as u32 == TAXA { return filtrado; }
        let n = filtrado.len();
        let mut saida = Vec::with_capacity((n as f64 / self.passo) as usize + 1);
        // `pos` é relativo ao bloco atual; -1 aponta para a última amostra do bloco anterior.
        loop {
            let i = self.pos.floor();
            if i + 1.0 >= n as f64 { break; }
            let f = (self.pos - i) as f32;
            let a = if i < 0.0 { self.anterior } else { filtrado[i as usize] };
            let b = filtrado[(i + 1.0) as usize];
            saida.push(a + (b - a) * f);
            self.pos += self.passo;
        }
        self.pos -= filtrado.len() as f64;
        if let Some(&u) = filtrado.last() { self.anterior = u; }
        saida
    }
}

pub fn nivel(amostras: &[f32]) -> f32 {
    if amostras.is_empty() { return 0.0; }
    let rms = (amostras.iter().map(|x| x * x).sum::<f32>() / amostras.len() as f32).sqrt();
    // Mesma curva aproximada da pílula do Mac: -50 dB a 0 dB em 0..1.
    ((20.0 * rms.max(1e-6).log10() + 50.0) / 50.0).clamp(0.0, 1.0)
}

/// Fluxo aberto na thread de áudio. Soltar para fechar.
pub struct Captura { id: Option<u64> }

impl Captura {
    /// `ao_bloco` recebe PCM 16 kHz mono e o nível (0..1) do bloco.
    pub fn iniciar(fonte: Fonte, microfone: &str, ao_bloco: impl FnMut(Vec<f32>, f32) + Send + 'static) -> Result<Self, Erro> {
        static PROXIMO: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let id = PROXIMO.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let nome = microfone.to_string();
        no_fio_de_audio(move || abrir(fonte, &nome, ao_bloco).map(|fluxo| FLUXOS.with(|f| { f.borrow_mut().insert(id, fluxo); })).map_err(|e| e.to_string()))?;
        Ok(Self { id: Some(id) })
    }

    pub fn parar(&mut self) {
        if let Some(id) = self.id.take() { no_fio_de_audio(move || FLUXOS.with(|f| { f.borrow_mut().remove(&id); })); }
    }
}

impl Drop for Captura { fn drop(&mut self) { self.parar(); } }

fn abrir(fonte: Fonte, microfone: &str, mut ao_bloco: impl FnMut(Vec<f32>, f32) + Send + 'static) -> Result<cpal::Stream, Erro> {
    let host = cpal::default_host();
    let (dispositivo, config) = match fonte {
        Fonte::Microfone => {
            let escolhido = if microfone.is_empty() { None } else {
                host.input_devices()?.find(|d| d.name().map(|n| n == microfone).unwrap_or(false))
            };
            let d = escolhido.or_else(|| host.default_input_device()).ok_or("nenhum microfone encontrado")?;
            let c = d.default_input_config()?;
            (d, c)
        }
        Fonte::Sistema => {
            let d = host.default_output_device().ok_or("nenhuma saída de áudio para capturar")?;
            let c = d.default_output_config()?;
            (d, c)
        }
    };
    let canais = config.channels() as usize;
    let mut reamostrar = Reamostrador::new(config.sample_rate().0);
    let mut acumulado: Vec<f32> = Vec::with_capacity(TAXA as usize / 5);
    let mut entregar = move |mono: Vec<f32>| {
        acumulado.extend(reamostrar.processar(&mono));
        if acumulado.len() >= TAXA as usize / 10 {
            let bloco = std::mem::take(&mut acumulado);
            let n = nivel(&bloco);
            ao_bloco(bloco, n);
        }
    };
    let erro = |e: cpal::StreamError| crate::caminhos::nota(format!("áudio: {e}"));
    let fluxo = match config.sample_format() {
        cpal::SampleFormat::F32 => dispositivo.build_input_stream(&config.config(), move |d: &[f32], _: &_| entregar(misturar(d, canais, |x| x)), erro, None)?,
        cpal::SampleFormat::I16 => dispositivo.build_input_stream(&config.config(), move |d: &[i16], _: &_| entregar(misturar(d, canais, |x| x as f32 / 32768.0)), erro, None)?,
        cpal::SampleFormat::U16 => dispositivo.build_input_stream(&config.config(), move |d: &[u16], _: &_| entregar(misturar(d, canais, |x| (x as f32 - 32768.0) / 32768.0)), erro, None)?,
        cpal::SampleFormat::I32 => dispositivo.build_input_stream(&config.config(), move |d: &[i32], _: &_| entregar(misturar(d, canais, |x| x as f32 / 2_147_483_648.0)), erro, None)?,
        outro => return Err(format!("formato de áudio não suportado: {outro:?}").into()),
    };
    fluxo.play()?;
    Ok(fluxo)
}

fn misturar<T: Copy>(dados: &[T], canais: usize, para_f32: impl Fn(T) -> f32) -> Vec<f32> {
    if canais <= 1 { return dados.iter().map(|&x| para_f32(x)).collect(); }
    dados.chunks(canais).map(|q| q.iter().map(|&x| para_f32(x)).sum::<f32>() / canais as f32).collect()
}

/// WAV 16 kHz mono 16 bits. O cabeçalho é reescrito a cada gravação: queda no meio ainda deixa um arquivo legível.
pub struct EscritorWav { arquivo: File, amostras: u32 }

impl EscritorWav {
    pub fn criar(caminho: &Path) -> io::Result<Self> {
        let mut e = Self { arquivo: File::create(caminho)?, amostras: 0 };
        e.cabecalho()?;
        Ok(e)
    }

    fn cabecalho(&mut self) -> io::Result<()> {
        let dados = self.amostras * 2;
        let mut c = Vec::with_capacity(44);
        c.extend_from_slice(b"RIFF"); c.extend_from_slice(&(36 + dados).to_le_bytes()); c.extend_from_slice(b"WAVEfmt ");
        c.extend_from_slice(&16u32.to_le_bytes()); c.extend_from_slice(&1u16.to_le_bytes()); c.extend_from_slice(&1u16.to_le_bytes());
        c.extend_from_slice(&TAXA.to_le_bytes()); c.extend_from_slice(&(TAXA * 2).to_le_bytes()); c.extend_from_slice(&2u16.to_le_bytes());
        c.extend_from_slice(&16u16.to_le_bytes()); c.extend_from_slice(b"data"); c.extend_from_slice(&dados.to_le_bytes());
        self.arquivo.seek(SeekFrom::Start(0))?;
        self.arquivo.write_all(&c)?;
        self.arquivo.seek(SeekFrom::End(0))?;
        Ok(())
    }

    pub fn gravar(&mut self, amostras: &[f32]) -> io::Result<()> {
        let bytes: Vec<u8> = amostras.iter().flat_map(|x| ((x.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes()).collect();
        self.arquivo.write_all(&bytes)?;
        self.amostras += amostras.len() as u32;
        self.cabecalho()
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    /// Sequência do crash no CI Windows (STATUS_ACCESS_VIOLATION): a primeira consulta de áudio
    /// numa thread que termina, e a seguinte em outra thread. No Windows, antes da thread de áudio
    /// própria, a segunda consulta usava o enumerador COM pendurado.
    #[test]
    fn consultas_de_audio_em_threads_que_terminam() {
        for _ in 0..3 {
            std::thread::spawn(|| { let _ = microfone_disponivel(); let _ = microfones(); }).join().unwrap();
        }
        let _ = microfone_disponivel();
        // Abrir e fechar captura também passa pela mesma thread (pode não haver microfone na máquina de testes).
        if let Ok(mut c) = Captura::iniciar(Fonte::Microfone, "", |_, _| {}) { c.parar(); }
        let _ = microfones();
    }

    #[test]
    fn reamostra_48k_para_16k_mantem_duracao() {
        let mut r = Reamostrador::new(48_000);
        let mut total = 0;
        for _ in 0..10 { total += r.processar(&vec![0.1; 4_800]).len(); }
        assert!((total as i64 - 16_000).abs() <= 2, "{total}");
    }

    #[test]
    fn reamostra_44k1_para_16k_mantem_duracao() {
        let mut r = Reamostrador::new(44_100);
        let total: usize = (0..10).map(|_| r.processar(&vec![0.0; 4_410]).len()).sum();
        assert!((total as i64 - 16_000).abs() <= 2, "{total}");
    }

    #[test]
    fn passa_baixa_atenua_acima_de_8k() {
        let mut r = Reamostrador::new(48_000);
        let tom: Vec<f32> = (0..48_000).map(|i| (2.0 * std::f32::consts::PI * 12_000.0 * i as f32 / 48_000.0).sin()).collect();
        let saida = r.processar(&tom);
        let rms = (saida[1000..].iter().map(|x| x * x).sum::<f32>() / (saida.len() - 1000) as f32).sqrt();
        assert!(rms < 0.15, "12 kHz vazou: {rms}");
    }

    #[test]
    fn wav_legivel_com_cabecalho_atualizado() {
        let p = std::env::temp_dir().join(format!("ditase-wav-{}.wav", std::process::id()));
        let mut w = EscritorWav::criar(&p).unwrap();
        w.gravar(&[0.5; 1600]).unwrap();
        w.gravar(&[0.0; 1600]).unwrap();
        let bytes = std::fs::read(&p).unwrap();
        assert_eq!(bytes.len(), 44 + 6400);
        assert_eq!(u32::from_le_bytes(bytes[40..44].try_into().unwrap()), 6400);
        let lido = crate::decodificacao::decodificar(&p).unwrap();
        assert_eq!(lido.len(), 3200);
        assert!((lido[0] - 0.5).abs() < 0.01);
        let _ = std::fs::remove_file(p);
    }
}
