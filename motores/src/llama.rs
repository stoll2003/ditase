//! Limpeza e resumo por llama-server local (llama.cpp oficial), porta do Swift
//! `Nucleo/Limpador.swift` (`LimpadorLlama`) e `Ditado/ServidorLLM.swift`.
//!
//! O servidor é processo filho em 127.0.0.1 numa porta livre, sobe sob demanda, espera o
//! `/health`, aquece o cache das instruções e é encerrado por ociosidade, por [`ServidorLlama::parar`]
//! e no `Drop`. No Windows fica num Job Object com "matar ao fechar": se o app cair, o
//! Windows encerra o filho junto; e sobe sem janela de console.
use crate::{erro, palavras, Erro, Limpador, PedidoLimpeza};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// Prompt de limpeza do app (mesmo arquivo do Mac).
pub const PROMPT_LIMPEZA: &str = include_str!("../../Recursos/prompt-limpeza.md");

// ---------------------------------------------------------------------------------------------
// Cliente (LimpadorLlama)

/// Limpador que fala com um llama-server já no ar pela API compatível com a da OpenAI.
pub struct LimpadorLlama {
    nome: String,
    url: String,
    instrucoes: String,
    agente: ureq::Agent,
}

impl LimpadorLlama {
    pub fn novo(porta: u16, modelo: &str, instrucoes: &str) -> Self {
        Self {
            nome: modelo.to_string(),
            url: format!("http://127.0.0.1:{porta}/v1/chat/completions"),
            instrucoes: instrucoes.to_string(),
            // Mesmo prazo do Swift (timeoutIntervalForRequest = 120). Sem proxy: só 127.0.0.1.
            agente: ureq::AgentBuilder::new()
                .timeout(Duration::from_secs(120))
                .try_proxy_from_env(false)
                .build(),
        }
    }

    pub fn montar_mensagem(p: &PedidoLimpeza) -> String {
        let mut m = String::new();
        if !p.vocabulario.is_empty() {
            let v: Vec<&str> = p.vocabulario.iter().take(60).map(String::as_str).collect();
            m += &format!("<vocabulario>{}</vocabulario>\n", v.join(", "));
        }
        if !p.anterior.is_empty() {
            m += &format!("<anterior>{}</anterior>\n", p.anterior);
        }
        m += &format!("<transcricao>{}</transcricao>", p.transcricao);
        m
    }

    /// Tira o que modelos pequenos às vezes põem em volta da resposta.
    pub fn extrair_resposta(bruta: &str) -> String {
        let mut t = bruta.to_string();
        while let Some(ini) = t.find("<think>") {
            let Some(rel) = t[ini + 7..].find("</think>") else {
                break;
            };
            t.replace_range(ini..ini + 7 + rel + 8, "");
        }
        let mut t = t.trim().to_string();
        for (abre, fecha) in [("<saida>", "</saida>"), ("<transcricao>", "</transcricao>")] {
            if let Some(resto) = t.strip_prefix(abre) {
                let mut r = resto.to_string();
                if let Some(f) = r.rfind(fecha) {
                    r.truncate(f);
                }
                t = r;
            }
        }
        t.trim().to_string()
    }

    pub fn corpo(&self, p: &PedidoLimpeza) -> serde_json::Value {
        let palavras = palavras::contar(&p.transcricao);
        serde_json::json!({
            "messages": [
                {"role": "system", "content": self.instrucoes},
                {"role": "user", "content": Self::montar_mensagem(p)},
            ],
            "temperature": 0,
            "max_tokens": (palavras * 4 + 96).min(4096),
            "cache_prompt": true,
            "stream": false,
        })
    }
}

impl Limpador for LimpadorLlama {
    fn nome(&self) -> &str {
        &self.nome
    }

    fn limpar(&self, p: &PedidoLimpeza) -> Result<String, Erro> {
        let resp = match self.agente.post(&self.url).send_json(self.corpo(p)) {
            Ok(r) => r,
            Err(ureq::Error::Status(s, _)) => {
                return Err(erro(format!("llama-server respondeu {s}")))
            }
            Err(e) => return Err(erro(format!("llama-server: {e}"))),
        };
        if resp.status() != 200 {
            return Err(erro(format!("llama-server respondeu {}", resp.status())));
        }
        let json: serde_json::Value = resp
            .into_json()
            .map_err(|_| erro("resposta sem conteúdo"))?;
        let escolha = &json["choices"][0];
        let conteudo = escolha["message"]["content"]
            .as_str()
            .ok_or_else(|| erro("resposta sem conteúdo"))?;
        if escolha["finish_reason"].as_str() == Some("length") {
            return Err(erro("resposta cortada pelo limite"));
        }
        Ok(Self::extrair_resposta(conteudo))
    }
}

// ---------------------------------------------------------------------------------------------
// Servidor (ServidorLLM)

#[derive(Debug, Clone, PartialEq)]
pub enum Estado {
    Desligado,
    Iniciando,
    Pronto,
    /// Não há llama-server no caminho indicado.
    SemBinario,
    /// O arquivo do modelo não existe.
    SemModelo(String),
    Falhou(String),
}

/// Ajustes do servidor. Os padrões são os do Mac.
#[derive(Debug, Clone)]
pub struct Ajustes {
    /// Minutos parado até desligar e devolver a memória (Swift: 3).
    pub ocioso: Duration,
    /// Prazo para o modelo carregar e o `/health` responder (Swift: 90 s).
    pub prazo_de_subida: Duration,
    pub contexto: u32,
    /// Threads da CPU; `None` deixa o llama.cpp decidir.
    pub threads: Option<u32>,
}

impl Default for Ajustes {
    fn default() -> Self {
        Self {
            ocioso: Duration::from_secs(180),
            prazo_de_subida: Duration::from_secs(90),
            contexto: 8192,
            threads: None,
        }
    }
}

type Observador = Box<dyn Fn(&Estado) + Send + Sync>;

struct Interno {
    processo: Option<Child>,
    porta: u16,
    modelo: Option<PathBuf>,
    estado: Estado,
    subindo: bool,
    ultimo_uso: Instant,
    /// Pedidos de limpeza em andamento: com algum ativo, a ociosidade não desliga o servidor.
    em_uso: usize,
    vigia: bool,
    #[cfg(windows)]
    job: Option<janela::Job>,
}

/// Mantém um llama-server local com o modelo de limpeza carregado.
pub struct ServidorLlama {
    binario: PathBuf,
    instrucoes: String,
    ajustes: Ajustes,
    interno: Mutex<Interno>,
    mudou: Condvar,
    ao_mudar: Mutex<Option<Observador>>,
}

impl ServidorLlama {
    /// `binario`: `llama-server` (Mac) ou `llama-server.exe` (Windows), na pasta com as DLLs.
    pub fn novo(binario: impl Into<PathBuf>, instrucoes: &str, ajustes: Ajustes) -> Arc<Self> {
        Arc::new(Self {
            binario: binario.into(),
            instrucoes: instrucoes.to_string(),
            ajustes,
            interno: Mutex::new(Interno {
                processo: None,
                porta: 0,
                modelo: None,
                estado: Estado::Desligado,
                subindo: false,
                ultimo_uso: Instant::now(),
                em_uso: 0,
                vigia: false,
                #[cfg(windows)]
                job: None,
            }),
            mudou: Condvar::new(),
            ao_mudar: Mutex::new(None),
        })
    }

    pub fn observar(&self, f: impl Fn(&Estado) + Send + Sync + 'static) {
        *self.ao_mudar.lock().unwrap() = Some(Box::new(f));
    }

    pub fn estado(&self) -> Estado {
        self.interno.lock().unwrap().estado.clone()
    }
    pub fn porta(&self) -> u16 {
        self.interno.lock().unwrap().porta
    }
    /// PID do processo filho, se estiver no ar.
    pub fn pid(&self) -> Option<u32> {
        self.interno
            .lock()
            .unwrap()
            .processo
            .as_ref()
            .map(|p| p.id())
    }

    fn mudar(&self, i: &mut Interno, novo: Estado) {
        i.estado = novo.clone();
        self.mudou.notify_all();
        if let Some(f) = self.ao_mudar.lock().unwrap().as_ref() {
            f(&novo);
        }
    }

    /// Limpador que garante o servidor no ar antes de cada pedido.
    pub fn limpador(self: &Arc<Self>, modelo: impl Into<PathBuf>) -> LimpadorDoServidor {
        LimpadorDoServidor::novo(self.clone(), modelo.into())
    }

    /// Garante o servidor no ar com o modelo pedido e devolve a porta.
    /// Chamadas simultâneas esperam a mesma subida.
    pub fn garantir(self: &Arc<Self>, modelo: &Path) -> Result<u16, Erro> {
        let mut i = self.interno.lock().unwrap();
        i.ultimo_uso = Instant::now();
        loop {
            let vivo = i
                .processo
                .as_mut()
                .map(|p| matches!(p.try_wait(), Ok(None)))
                .unwrap_or(false);
            if i.estado == Estado::Pronto && vivo && i.modelo.as_deref() == Some(modelo) {
                return Ok(i.porta);
            }
            if !i.subindo {
                break;
            }
            i = self.mudou.wait(i).unwrap();
            if !i.subindo && i.modelo.as_deref() == Some(modelo) {
                return match &i.estado {
                    Estado::Pronto => Ok(i.porta),
                    e => Err(erro(format!("modelo de limpeza indisponível: {e:?}"))),
                };
            }
        }
        i.subindo = true;
        i.modelo = Some(modelo.to_path_buf());
        Self::encerrar(&mut i);
        drop(i);
        let r = self.subir(modelo);
        let mut i = self.interno.lock().unwrap();
        i.subindo = false;
        match r {
            Ok(porta) => {
                // O prazo de ociosidade conta a partir de agora, não do começo da subida.
                i.ultimo_uso = Instant::now();
                self.mudar(&mut i, Estado::Pronto);
                if !i.vigia {
                    i.vigia = true;
                    self.iniciar_vigia();
                }
                Ok(porta)
            }
            Err(e) => {
                Self::encerrar(&mut i);
                let estado = match &e {
                    Falha::SemBinario => Estado::SemBinario,
                    Falha::SemModelo(m) => Estado::SemModelo(m.clone()),
                    Falha::Outra(m) => Estado::Falhou(m.clone()),
                };
                self.mudar(&mut i, estado);
                Err(erro(format!("modelo de limpeza indisponível: {e}")))
            }
        }
    }

    fn subir(&self, modelo: &Path) -> Result<u16, Falha> {
        if !self.binario.is_file() {
            return Err(Falha::SemBinario);
        }
        if !modelo.is_file() {
            return Err(Falha::SemModelo(modelo.display().to_string()));
        }
        {
            let mut i = self.interno.lock().unwrap();
            self.mudar(&mut i, Estado::Iniciando);
        }
        let porta = porta_livre().map_err(|e| Falha::Outra(format!("sem porta livre: {e}")))?;
        // Caminho absoluto: com current_dir, um caminho relativo seria resolvido na pasta nova.
        let binario = absoluto(&self.binario).map_err(|e| Falha::Outra(format!("binário: {e}")))?;
        let modelo = &absoluto(modelo).map_err(|e| Falha::Outra(format!("modelo: {e}")))?;
        let mut cmd = Command::new(&binario);
        cmd.arg("-m")
            .arg(modelo)
            .args(["--host", "127.0.0.1", "--port", &porta.to_string()])
            .args([
                "-c",
                &self.ajustes.contexto.to_string(),
                "-ngl",
                "99",
                "-np",
                "1",
                "--jinja",
                "--reasoning-budget",
                "0",
                "--no-slots",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if let Some(t) = self.ajustes.threads {
            cmd.args(["-t", &t.to_string()]);
        }
        if let Some(pasta) = binario.parent() {
            cmd.current_dir(pasta); // DLLs ao lado do .exe
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let filho = cmd
            .spawn()
            .map_err(|e| Falha::Outra(format!("não iniciou: {e}")))?;
        {
            let mut i = self.interno.lock().unwrap();
            #[cfg(windows)]
            {
                i.job = janela::Job::prender(&filho);
            }
            i.processo = Some(filho);
            i.porta = porta;
        }

        let saude = format!("http://127.0.0.1:{porta}/health");
        let agente = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(2))
            .try_proxy_from_env(false)
            .build();
        let prazo = Instant::now() + self.ajustes.prazo_de_subida;
        while Instant::now() < prazo {
            {
                let mut i = self.interno.lock().unwrap();
                let vivo = i
                    .processo
                    .as_mut()
                    .map(|p| matches!(p.try_wait(), Ok(None)))
                    .unwrap_or(false);
                if !vivo {
                    return Err(Falha::Outra(
                        "o servidor encerrou ao carregar o modelo".into(),
                    ));
                }
            }
            if matches!(agente.get(&saude).call(), Ok(r) if r.status() == 200) {
                // Primeira chamada deixa as instruções em cache: os ditados só pagam o texto novo.
                let nome = nome_do_modelo(modelo);
                let _ =
                    LimpadorLlama::novo(porta, &nome, &self.instrucoes).limpar(&PedidoLimpeza {
                        transcricao: "teste de aquecimento do modelo".into(),
                        ..Default::default()
                    });
                return Ok(porta);
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        Err(Falha::Outra("o modelo demorou demais para carregar".into()))
    }

    /// Para o servidor e devolve a memória.
    pub fn parar(&self) {
        let mut i = self.interno.lock().unwrap();
        Self::encerrar(&mut i);
        if !i.subindo {
            self.mudar(&mut i, Estado::Desligado);
        }
    }

    fn encerrar(i: &mut Interno) {
        if let Some(mut p) = i.processo.take() {
            encerrar_processo(&mut p);
        }
        #[cfg(windows)]
        {
            i.job = None;
        }
    }

    fn iniciar_vigia(self: &Arc<Self>) {
        let fraco = Arc::downgrade(self);
        let passo =
            (self.ajustes.ocioso / 6).clamp(Duration::from_millis(50), Duration::from_secs(5));
        std::thread::spawn(move || loop {
            std::thread::sleep(passo);
            let Some(s) = fraco.upgrade() else { return };
            let ocioso = {
                let i = s.interno.lock().unwrap();
                i.processo.is_some()
                    && !i.subindo
                    && i.em_uso == 0
                    && i.ultimo_uso.elapsed() >= s.ajustes.ocioso
            };
            if ocioso {
                s.parar();
            }
        });
    }

    fn entrar(&self) {
        let mut i = self.interno.lock().unwrap();
        i.em_uso += 1;
        i.ultimo_uso = Instant::now();
    }

    fn sair(&self) {
        let mut i = self.interno.lock().unwrap();
        i.em_uso = i.em_uso.saturating_sub(1);
        i.ultimo_uso = Instant::now();
    }
}

impl Drop for ServidorLlama {
    fn drop(&mut self) {
        if let Ok(mut i) = self.interno.lock() {
            Self::encerrar(&mut i);
        }
    }
}

#[derive(Debug)]
enum Falha {
    SemBinario,
    SemModelo(String),
    Outra(String),
}

impl std::fmt::Display for Falha {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Falha::SemBinario => write!(f, "llama-server ausente"),
            Falha::SemModelo(m) => write!(f, "modelo ausente: {m}"),
            Falha::Outra(m) => write!(f, "{m}"),
        }
    }
}

/// Encerra com pedido educado e, se não sair em 2 s, à força (como o Swift).
fn encerrar_processo(p: &mut Child) {
    if !matches!(p.try_wait(), Ok(None)) {
        return;
    }
    #[cfg(unix)]
    unsafe {
        libc::kill(p.id() as i32, libc::SIGTERM);
    }
    #[cfg(unix)]
    {
        let prazo = Instant::now() + Duration::from_secs(2);
        while Instant::now() < prazo {
            if !matches!(p.try_wait(), Ok(None)) {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    // No Windows não há SIGTERM para processo sem janela: TerminateProcess direto.
    let _ = p.kill();
    let _ = p.wait();
}

/// Caminho absoluto sem o prefixo `\\?\` que o `canonicalize` põe no Windows (alguns
/// programas não aceitam).
fn absoluto(p: &Path) -> std::io::Result<PathBuf> {
    let c = std::fs::canonicalize(p)?;
    let s = c.to_string_lossy();
    match s.strip_prefix(r"\\?\") {
        Some(resto) if !resto.starts_with("UNC") => Ok(PathBuf::from(resto)),
        _ => Ok(c),
    }
}

/// Porta livre em 127.0.0.1 (o sistema escolhe).
pub fn porta_livre() -> std::io::Result<u16> {
    let l = std::net::TcpListener::bind(("127.0.0.1", 0))?;
    Ok(l.local_addr()?.port())
}

fn nome_do_modelo(modelo: &Path) -> String {
    modelo
        .file_name()
        .map(|n| n.to_string_lossy().replace(".gguf", ""))
        .unwrap_or_default()
}

/// Espera o servidor subir (ele pode começar a carregar quando a gravação começa) e repassa.
pub struct LimpadorDoServidor {
    servidor: Arc<ServidorLlama>,
    modelo: PathBuf,
    nome: String,
}

impl LimpadorDoServidor {
    fn novo(servidor: Arc<ServidorLlama>, modelo: PathBuf) -> Self {
        let nome = nome_do_modelo(&modelo);
        Self {
            servidor,
            modelo,
            nome,
        }
    }
}

impl Limpador for LimpadorDoServidor {
    fn nome(&self) -> &str {
        &self.nome
    }

    fn limpar(&self, p: &PedidoLimpeza) -> Result<String, Erro> {
        self.servidor.entrar();
        let r = self.servidor.garantir(&self.modelo).and_then(|porta| {
            LimpadorLlama::novo(porta, &self.nome, &self.servidor.instrucoes).limpar(p)
        });
        self.servidor.sair();
        r
    }
}

#[cfg(windows)]
mod janela {
    //! Job Object com JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: quando o último handle fecha (inclusive
    //! se o app cair), o Windows encerra o llama-server.
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::*;

    pub struct Job(HANDLE);
    unsafe impl Send for Job {}

    impl Job {
        pub fn prender(filho: &std::process::Child) -> Option<Job> {
            unsafe {
                let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if job.is_null() {
                    return None;
                }
                let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                let ok = SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as *const _,
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                ) != 0
                    && AssignProcessToJobObject(job, filho.as_raw_handle() as HANDLE) != 0;
                if ok {
                    Some(Job(job))
                } else {
                    CloseHandle(job);
                    None
                }
            }
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.0) };
        }
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn mensagem_igual_ao_swift() {
        let p = PedidoLimpeza {
            transcricao: "oi tudo bem".into(),
            anterior: "Bom dia.".into(),
            vocabulario: vec!["Dita.se".into(), "Sailm".into()],
        };
        assert_eq!(
            LimpadorLlama::montar_mensagem(&p),
            "<vocabulario>Dita.se, Sailm</vocabulario>\n<anterior>Bom dia.</anterior>\n<transcricao>oi tudo bem</transcricao>"
        );
        let so = PedidoLimpeza {
            transcricao: "x".into(),
            ..Default::default()
        };
        assert_eq!(
            LimpadorLlama::montar_mensagem(&so),
            "<transcricao>x</transcricao>"
        );
        let muitos = PedidoLimpeza {
            transcricao: "x".into(),
            vocabulario: (0..80).map(|i| i.to_string()).collect(),
            ..Default::default()
        };
        assert!(LimpadorLlama::montar_mensagem(&muitos).contains(", 59</vocabulario>"));
    }

    #[test]
    fn extrai_resposta_como_o_swift() {
        assert_eq!(
            LimpadorLlama::extrair_resposta("<think>hmm</think>\n Olá. "),
            "Olá."
        );
        assert_eq!(
            LimpadorLlama::extrair_resposta("<think>a</think><think>b</think>x"),
            "x"
        );
        assert_eq!(
            LimpadorLlama::extrair_resposta("<saida>Texto.</saida>"),
            "Texto."
        );
        assert_eq!(
            LimpadorLlama::extrair_resposta("<transcricao>T</transcricao> "),
            "T"
        );
        assert_eq!(LimpadorLlama::extrair_resposta("<saida>sem fim"), "sem fim");
        assert_eq!(
            LimpadorLlama::extrair_resposta("<think>aberto"),
            "<think>aberto"
        );
    }

    #[test]
    fn corpo_do_pedido() {
        let l = LimpadorLlama::novo(1, "m", "inst");
        let c = l.corpo(&PedidoLimpeza {
            transcricao: "um dois três".into(),
            ..Default::default()
        });
        assert_eq!(c["max_tokens"], 108);
        assert_eq!(c["temperature"], 0);
        assert_eq!(c["cache_prompt"], true);
        assert_eq!(c["messages"][0]["content"], "inst");
        let longo = "palavra ".repeat(2000);
        assert_eq!(
            l.corpo(&PedidoLimpeza {
                transcricao: longo,
                ..Default::default()
            })["max_tokens"],
            4096
        );
    }

    #[test]
    fn prompt_embutido() {
        assert!(PROMPT_LIMPEZA.contains("<transcricao>"));
    }

    #[test]
    fn sem_binario_ou_sem_modelo() {
        let s = ServidorLlama::novo("/nao/existe/llama-server", "x", Ajustes::default());
        assert!(s.garantir(Path::new("/tmp/nada.gguf")).is_err());
        assert_eq!(s.estado(), Estado::SemBinario);
        let bin = std::env::current_exe().unwrap();
        let s = ServidorLlama::novo(bin, "x", Ajustes::default());
        assert!(s.garantir(Path::new("/nao/existe.gguf")).is_err());
        assert!(matches!(s.estado(), Estado::SemModelo(_)));
    }

    #[test]
    fn porta_livre_responde() {
        assert!(porta_livre().unwrap() > 0);
    }
}
