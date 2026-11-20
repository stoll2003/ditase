//! TLS com uma única raiz privada por partida. Mesmo se a porta for reutilizada, o novo
//! ocupante não recebe o cabeçalho de autorização nem o corpo de uma requisição.
use crate::{erro, Erro};
use std::io::{BufRead, BufReader, Write};
use std::process::{ChildStderr, Command};
use std::sync::{mpsc, Arc};
use std::time::Duration;

pub(crate) struct Identidade {
    chave_tls: tempfile::NamedTempFile,
    certificado: tempfile::NamedTempFile,
    pub canal: Canal,
}

#[derive(Clone)]
pub(crate) struct Canal {
    agente: ureq::Agent,
    segredo: String,
}

impl Identidade {
    pub fn nova() -> Result<Self, Erro> {
        let par = rcgen::generate_simple_self_signed(vec!["127.0.0.1".to_string()])?;
        let mut certificado = tempfile::NamedTempFile::new()?;
        certificado.write_all(par.cert.pem().as_bytes())?;
        let mut chave_tls = tempfile::NamedTempFile::new()?;
        chave_tls.write_all(par.key_pair.serialize_pem().as_bytes())?;
        let mut raizes = rustls::RootCertStore::empty();
        raizes.add(par.cert.der().clone())?;
        let tls = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()?
        .with_root_certificates(raizes)
        .with_no_client_auth();
        let agente = ureq::AgentBuilder::new()
            .tls_config(Arc::new(tls))
            .try_proxy_from_env(false)
            .redirects(0)
            .build();
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).map_err(|_| erro("fonte de aleatoriedade indisponível"))?;
        let segredo = bytes.iter().map(|b| format!("{b:02x}")).collect();
        Ok(Self {
            chave_tls,
            certificado,
            canal: Canal { agente, segredo },
        })
    }

    pub fn configurar(&self, cmd: &mut Command) -> Result<(), Erro> {
        cmd.env_remove("LLAMA_ARG_API_KEY_FILE")
            .env_remove("LLAMA_ARG_LOG_FILE")
            .env_remove("LLAMA_ARG_ENDPOINT_METRICS")
            .env_remove("LLAMA_ARG_ENDPOINT_PROPS")
            .env("LLAMA_API_KEY", &self.canal.segredo)
            .arg("--ssl-key-file")
            .arg(crate::audio::caminho_c(self.chave_tls.path())?)
            .arg("--ssl-cert-file")
            .arg(crate::audio::caminho_c(self.certificado.path())?);
        Ok(())
    }
}

impl Canal {
    pub fn pronto(&self, porta: u16, prazo: Duration) -> bool {
        let url = format!("https://127.0.0.1:{porta}/health");
        matches!(self.agente.get(&url).timeout(prazo).set("Authorization", &format!("Bearer {}", self.segredo)).call(), Ok(r) if r.status() == 200)
    }

    pub fn exige_chave(&self, porta: u16, prazo: Duration) -> bool {
        let url = format!("https://127.0.0.1:{porta}/v1/models");
        matches!(
            self.agente.get(&url).timeout(prazo).call(),
            Err(ureq::Error::Status(401, _))
        )
    }

    pub fn validar_chave(&self, porta: u16, prazo: Duration) -> Result<(), Erro> {
        let resposta = self
            .agente
            .get(&format!("https://127.0.0.1:{porta}/v1/models"))
            .timeout(prazo)
            .set("Authorization", &format!("Bearer {}", self.segredo))
            .call()?;
        if resposta.status() != 200 {
            return Err(erro("credencial recusada"));
        }
        Ok(())
    }

    pub fn completar(
        &self,
        porta: u16,
        corpo: &serde_json::Value,
        prazo: Duration,
    ) -> Result<serde_json::Value, Erro> {
        let url = format!("https://127.0.0.1:{porta}/v1/chat/completions");
        let resp = self
            .agente
            .post(&url)
            .timeout(prazo)
            .set("Authorization", &format!("Bearer {}", self.segredo))
            .send_json(corpo)
            .map_err(|e| erro(format!("llama-server: {e}")))?;
        if resp.status() != 200 {
            return Err(erro(format!("llama-server respondeu {}", resp.status())));
        }
        resp.into_json().map_err(|_| erro("resposta sem conteúdo"))
    }
}

/// A porta vem do stderr privado do filho depois do bind. Depois da primeira indicação,
/// o restante do pipe é drenado e descartado, sem guardar linhas de pedidos ou respostas.
pub(crate) fn escutar_porta(stderr: ChildStderr) -> std::io::Result<mpsc::Receiver<u16>> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("llama.porta".into())
        .spawn(move || {
            let mut leitor = BufReader::new(stderr);
            let mut linha = Vec::new();
            while let Ok(n) = leitor.read_until(b'\n', &mut linha) {
                if n == 0 {
                    break;
                }
                if let Some(porta) = String::from_utf8_lossy(&linha)
                    .split_once("listening on https://127.0.0.1:")
                    .and_then(|(_, fim)| fim.trim().parse::<u16>().ok())
                    .filter(|p| *p != 0)
                {
                    let _ = tx.send(porta);
                    break;
                }
                linha.clear();
            }
            drop(tx);
            let _ = std::io::copy(&mut leitor, &mut std::io::sink());
        })?;
    Ok(rx)
}

#[cfg(test)]
mod testes {
    use super::*;
    use rustls::pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer};
    use std::net::TcpListener;

    // Responde uma única vez. O marcador só conta se chegou como HTTP dentro do túnel TLS.
    fn servidor(
        id: &Identidade,
        resposta: impl Into<String>,
    ) -> (u16, std::thread::JoinHandle<bool>) {
        let resposta = resposta.into();
        let cert = CertificateDer::from_pem_file(id.certificado.path()).unwrap();
        let chave = PrivateKeyDer::from_pem_file(id.chave_tls.path()).unwrap();
        let config = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![cert], chave)
        .unwrap();
        let esperado = format!("Authorization: Bearer {}", id.canal.segredo).to_ascii_lowercase();
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let porta = listener.local_addr().unwrap().port();
        let filho = std::thread::spawn(move || {
            let (socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let tls = rustls::ServerConnection::new(Arc::new(config)).unwrap();
            let mut canal = rustls::StreamOwned::new(tls, socket);
            let mut recebeu = false;
            let mut leitor = BufReader::new(&mut canal);
            loop {
                let mut linha = String::new();
                match leitor.read_line(&mut linha) {
                    Ok(0) | Err(_) => return false,
                    Ok(_) if linha == "\r\n" => break,
                    _ => recebeu |= linha.trim_end().to_ascii_lowercase() == esperado,
                }
            }
            let _ = canal.write_all(resposta.as_bytes());
            let _ = canal.flush();
            recebeu
        });
        (porta, filho)
    }

    #[test]
    fn so_envia_autorizacao_para_o_certificado_da_partida() {
        let cert = Identidade::nova().unwrap();
        let outra = Identidade::nova().unwrap();
        let resposta = "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}";
        let (porta, servidor_atual) = servidor(&cert, resposta);
        let prazo = Duration::from_secs(3);
        assert!(cert
            .canal
            .completar(porta, &serde_json::json!({}), prazo)
            .is_ok());
        assert!(servidor_atual.join().unwrap());
        let (porta, servidor_anterior) = servidor(&cert, resposta);
        assert!(outra
            .canal
            .completar(porta, &serde_json::json!({}), prazo)
            .is_err());
        assert!(!servidor_anterior.join().unwrap());
    }

    #[test]
    fn recusa_redirecionamento_de_saude() {
        let cert = Identidade::nova().unwrap();
        let destino = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        destino.set_nonblocking(true).unwrap();
        let redir = format!("HTTP/1.1 302 Found\r\nLocation: http://{}/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n", destino.local_addr().unwrap());
        let (porta, filho) = servidor(&cert, redir);
        assert!(!cert.canal.pronto(porta, Duration::from_secs(1)));
        assert!(filho.join().unwrap());
        assert_eq!(
            destino.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }
}
