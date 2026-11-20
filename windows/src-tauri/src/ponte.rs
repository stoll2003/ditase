//! Ponte entre a interface e o app (porta de `Ponte.swift`). A interface chama
//! `invoke("ponte", {metodo, params})` pelo adaptador de `ui/src/lib/ponte.ts` e recebe o mesmo
//! JSON em texto que o Mac devolve. Os avisos à interface usam o mesmo `CustomEvent` do Mac.
use std::path::PathBuf;

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, EventTarget};
use tauri_plugin_dialog::DialogExt;

use std::sync::Arc;

use crate::{audio, caminhos, decodificacao, integracao::Integracao, lancamento::Lancamento, nativo, preferencias::Tecla, Casca};

/// Situação de cada método do contrato no Windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Situacao {
    /// Implementado só na casca.
    Pronto,
    /// Ligado ao núcleo e aos motores (`integracao`, `lancamento`, `motores_locais`).
    Nucleo,
    /// Sem equivalente no Windows: devolve erro claro.
    Indisponivel,
}

/// Todos os métodos atendidos, com aliases. Os 41 do contrato aparecem aqui, mais `relatorio.copiar`.
pub const METODOS: &[(&str, Situacao)] = &[
    ("estado", Situacao::Pronto),
    ("ajustes.ler", Situacao::Pronto),
    ("ajustes.definir", Situacao::Pronto),
    ("permissao.pedir", Situacao::Pronto),
    ("copiar", Situacao::Pronto),
    ("gravacao.alternar", Situacao::Pronto),
    ("abrir.pastaDeModelos", Situacao::Pronto),
    ("lancamento.exportar", Situacao::Pronto),
    ("lancamento.permissao", Situacao::Pronto),
    ("relatorio.copiar", Situacao::Pronto),
    ("historico", Situacao::Nucleo),
    ("estatisticas", Situacao::Nucleo),
    ("dicionario.listar", Situacao::Nucleo),
    ("dicionario.salvar", Situacao::Nucleo),
    ("dicionario.apagar", Situacao::Nucleo),
    ("ditado.apagar", Situacao::Nucleo),
    ("ditado.retranscrever", Situacao::Nucleo),
    ("wispr.importar", Situacao::Indisponivel),
    ("modelo.baixar", Situacao::Nucleo),
    ("modelo.cancelar", Situacao::Nucleo),
    ("lancamento.estado", Situacao::Nucleo),
    ("lancamento.documento", Situacao::Nucleo),
    ("lancamento.buscarArquivos", Situacao::Nucleo),
    ("lancamento.recuperar", Situacao::Nucleo),
    ("lancamento.descartar", Situacao::Nucleo),
    ("lancamento.escolher", Situacao::Nucleo),
    ("lancamento.adicionar", Situacao::Nucleo),
    ("arquivo.transcrever", Situacao::Nucleo),
    ("lancamento.repetir", Situacao::Nucleo),
    ("arquivo.listar", Situacao::Nucleo),
    ("reuniao.listar", Situacao::Nucleo),
    ("arquivo.cancelar", Situacao::Nucleo),
    ("lancamento.remover", Situacao::Nucleo),
    ("lancamento.desfazer", Situacao::Nucleo),
    ("lancamento.cancelarFila", Situacao::Nucleo),
    ("lancamento.iniciar", Situacao::Nucleo),
    ("reuniao.iniciar", Situacao::Nucleo),
    ("lancamento.pausar", Situacao::Nucleo),
    ("reuniao.pausar", Situacao::Nucleo),
    ("lancamento.encerrar", Situacao::Nucleo),
    ("reuniao.encerrar", Situacao::Nucleo),
    ("lancamento.resumir", Situacao::Nucleo),
    ("lancamento.cancelarResumo", Situacao::Nucleo),
    ("lancamento.titulo", Situacao::Nucleo),
    ("lancamento.voz", Situacao::Nucleo),
    ("falante.renomear", Situacao::Nucleo),
    ("lancamento.trecho", Situacao::Nucleo),
];

type R = Result<String, String>;

fn json_de(v: impl serde::Serialize) -> R { serde_json::to_string(&v).map_err(|e| e.to_string()) }
fn erro(e: impl std::fmt::Display) -> String { e.to_string() }
fn ok() -> R { Ok("true".into()) }

fn texto(p: &Value, chave: &str) -> Result<String, String> {
    p.get(chave).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(String::from).ok_or_else(|| format!("falta {chave}"))
}
fn inteiro(p: &Value, chave: &str) -> Result<i64, String> {
    p.get(chave).and_then(Value::as_f64).map(|n| n as i64).ok_or_else(|| format!("falta {chave}"))
}
fn prefixo(s: &str, n: usize) -> String { s.chars().take(n).collect() }

/// Avisa a interface de que um assunto mudou (`window` recebe `CustomEvent('ditado', {detail})`).
pub fn emitir(app: &AppHandle, assunto: &str) { emitir_evento(app, "ditado", &json!(assunto)); }

/// Eventos com dados (`ditado:progresso`, `ditado:trechos`, `ditado:resultado`, `ditado:arquivos-soltos`).
/// Evento estruturado só para a janela principal (sem avaliar JavaScript): o adaptador de
/// `ui/src/lib/ponte.ts` repassa como o mesmo `CustomEvent` do Mac. Nomes fora de `EVENTOS` não saem.
pub fn emitir_evento(app: &AppHandle, evento: &str, detalhe: &Value) {
    if !EVENTOS.contains(&evento) { return; }
    let _ = app.emit_to(EventTarget::WebviewWindow { label: "principal".into() }, evento, detalhe);
}

/// Eventos que a interface escuta (mesma lista em `ui/src/lib/ponte.ts`).
pub const EVENTOS: &[&str] = &["ditado", "ditado:progresso", "ditado:trechos", "ditado:resultado", "ditado:intervalos", "ditado:arquivos-soltos"];

fn escolher_arquivos(app: &AppHandle, varios: bool) -> Vec<PathBuf> {
    let dialogo = app.dialog().file().set_title("Escolher áudio ou vídeo").add_filter("Áudio e vídeo", decodificacao::SUPORTADOS);
    let escolhidos = if varios { dialogo.blocking_pick_files().unwrap_or_default() } else { dialogo.blocking_pick_file().into_iter().collect() };
    escolhidos.into_iter().filter_map(|f| f.into_path().ok()).collect()
}

/// Atende uma chamada. Roda fora da thread principal (os seletores nativos bloqueiam).
pub fn atender(casca: &Casca, metodo: &str, p: &Value) -> R {
    let app = &casca.app;
    let banco = &casca.integracao;
    let lance = &casca.lancamento;
    let c = &casca.controlador;
    if let Some(r) = atender_dados(banco, lance, metodo, p) { return r; }
    match metodo {
        // MARK: Arquivos e reuniões (os que precisam da janela ou do sistema)
        "lancamento.escolher" | "lancamento.adicionar" | "arquivo.transcrever" => {
            let arquivos = escolher_arquivos(app, true);
            if arquivos.is_empty() { return Ok("null".into()); }
            lance.adicionar(&arquivos).map_err(erro).and_then(json_de)
        }
        "lancamento.repetir" => {
            let id = texto(p, "id")?;
            let fonte = match lance.fonte_para_repetir(&id).map_err(erro)? {
                Some(f) => f,
                None => match escolher_arquivos(app, false).into_iter().next() { Some(f) => f, None => return Ok("false".into()) },
            };
            lance.repetir(&id, &fonte).map_err(erro).and_then(|_| ok())
        }
        "lancamento.exportar" => {
            let (nome, formato, conteudo) = (texto(p, "nome")?, texto(p, "formato")?, p.get("texto").and_then(Value::as_str).ok_or("falta texto")?.to_string());
            if !["txt", "md", "srt", "vtt"].contains(&formato.as_str()) { return Err("formato de exportação inválido".into()); }
            let base = std::path::Path::new(&nome).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or(nome.clone());
            let destino = app.dialog().file().set_file_name(&base).add_filter(formato.to_uppercase(), &[formato.as_str()]).blocking_save_file();
            let Some(destino) = destino.and_then(|d| d.into_path().ok()) else { return Ok("false".into()) };
            std::fs::write(destino, conteudo).map_err(erro)?;
            ok()
        }
        "lancamento.permissao" => {
            // Áudio do sistema não pede permissão no Windows; o microfone abre a tela de Privacidade.
            match texto(p, "qual")?.as_str() {
                "audioSistema" => ok(),
                "microfone" => { let _ = nativo::sistema::abrir("ms-settings:privacy-microphone"); ok() }
                _ => Err("permissão desconhecida".into()),
            }
        }

        // MARK: Ditado
        "estado" => json_de(estado(casca)),
        "ajustes.ler" => json_de(ajustes(casca)),
        "ajustes.definir" => {
            let (chave, valor) = match (p.get("chave").and_then(Value::as_str), p.get("valor").and_then(Value::as_str)) {
                (Some(k), Some(v)) => (k, v),
                _ => return Err("falta chave ou valor".into()),
            };
            if chave == "atalho" && Tecla::de_id(valor).is_none() { return Err("tecla desconhecida".into()); }
            if chave == "modelo" && !crate::motores_locais::modelos_do_catalogo().contains(&valor) { return Err("modelo fora do catálogo".into()); }
            if casca.prefs.definir(chave, valor) {
                if chave == "iniciarComWindows" { crate::iniciar_com_windows(app, valor == "1"); }
                c.preferencia_mudou(chave);
            }
            ok()
        }
        "permissao.pedir" => match p.get("qual").and_then(Value::as_str) {
            Some("microfone") => { let _ = nativo::sistema::abrir("ms-settings:privacy-microphone"); emitir(app, "estado"); ok() }
            // O gancho de teclado não precisa de permissão no Windows.
            Some("acessibilidade") => ok(),
            _ => Err("permissão desconhecida".into()),
        },
        "ditado.apagar" => {
            let id = inteiro(p, "id")?;
            if let Ok(Some(nome)) = banco.audio_do_ditado(id) {
                if !nome.is_empty() && !nome.contains(['/', '\\']) { let _ = std::fs::remove_file(caminhos::audios().join(nome)); }
            }
            banco.apagar_ditado(id).map_err(erro)?;
            emitir(app, "historico");
            ok()
        }
        "ditado.retranscrever" => { c.retranscrever(inteiro(p, "id")?)?; ok() }
        "copiar" => { nativo::colagem::copiar(p.get("texto").and_then(Value::as_str).ok_or("falta texto")?)?; ok() }
        "gravacao.alternar" => { c.alternar(); ok() }
        "wispr.importar" => {
            let resumo = banco.importar_wispr().map_err(erro)?;
            emitir(app, "historico");
            emitir(app, "dicionario");
            json_de(resumo)
        }
        "modelo.baixar" => { banco.motores.baixar_modelo()?; emitir(app, "estado"); ok() }
        "modelo.cancelar" => { banco.motores.cancelar_modelo(); ok() }
        "abrir.pastaDeModelos" => { nativo::sistema::abrir(&caminhos::modelos().to_string_lossy()).map_err(erro)?; ok() }
        "relatorio.copiar" => {
            let texto = crate::relatorio::montar(casca);
            nativo::colagem::copiar(&texto)?;
            json_de(texto)
        }
        _ => Err(format!("método desconhecido: {metodo}")),
    }
}

/// Métodos que só dependem do núcleo e dos motores, sem janela nem diálogo: testáveis com dublês.
pub fn atender_dados(banco: &Integracao, lance: &Arc<Lancamento>, metodo: &str, p: &Value) -> Option<R> {
    const OUTRO: &str = "\u{0}outro";
    let r = dados(banco, lance, metodo, p, OUTRO);
    if matches!(&r, Err(e) if e == OUTRO) { None } else { Some(r) }
}

fn dados(banco: &Integracao, lance: &Arc<Lancamento>, metodo: &str, p: &Value, outro: &str) -> R {
    match metodo {
        "lancamento.estado" => lance.estado(audio::microfone_disponivel()).map_err(erro).and_then(json_de),
        "lancamento.documento" => lance.documento(&texto(p, "id")?).map_err(erro).and_then(json_de),
        "lancamento.buscarArquivos" => {
            let termo = p.get("termo").and_then(Value::as_str).unwrap_or("").trim();
            if termo.is_empty() { Ok("[]".into()) } else { lance.procurar_arquivos(termo).map_err(erro).and_then(json_de) }
        }
        "lancamento.recuperar" => lance.recuperar_reuniao(&texto(p, "id")?).map_err(erro).and_then(|_| ok()),
        "lancamento.descartar" => lance.descartar_reuniao(&texto(p, "id")?).map_err(erro).and_then(|_| ok()),
        "arquivo.listar" => lance.estado(true).map_err(erro).and_then(|e| json_de(&e["arquivos"])),
        "reuniao.listar" => lance.estado(true).map_err(erro).and_then(|e| json_de(&e["anteriores"])),
        "arquivo.cancelar" => lance.cancelar(&texto(p, "id")?).map_err(erro).and_then(|_| ok()),
        "lancamento.remover" => lance.remover(&texto(p, "id")?).map_err(erro).and_then(|_| ok()),
        "lancamento.desfazer" => lance.desfazer(&texto(p, "id")?).map_err(erro).and_then(|_| ok()),
        "lancamento.cancelarFila" => lance.cancelar_fila().map_err(erro).and_then(|_| ok()),
        "reuniao.iniciar" | "lancamento.iniciar" => {
            // A interface manda `mac`/`macEMicrofone`; no Windows a fonte é o áudio do sistema (loopback).
            let fonte = texto(p, "fonte")?;
            if !["mac", "macEMicrofone", "sistema", "sistemaEMicrofone"].contains(&fonte.as_str()) { return Err("fonte desconhecida".into()); }
            lance.iniciar_reuniao(&fonte).map_err(erro).and_then(|_| ok())
        }
        "reuniao.pausar" | "lancamento.pausar" => lance.pausar(&texto(p, "id")?).map_err(erro).and_then(|_| ok()),
        "reuniao.encerrar" | "lancamento.encerrar" => lance.encerrar(&texto(p, "id")?).map_err(erro).and_then(|_| ok()),
        "lancamento.resumir" => lance.resumir(&texto(p, "id")?).map_err(erro).and_then(|_| ok()),
        "lancamento.cancelarResumo" => { lance.cancelar_resumo(&texto(p, "id")?); ok() }
        "lancamento.titulo" => lance.alterar_titulo(&texto(p, "id")?, &prefixo(&texto(p, "nome")?, 160)).map_err(erro).and_then(|_| ok()),
        "falante.renomear" | "lancamento.voz" => lance.renomear_voz(&texto(p, "id")?, &texto(p, "voz")?, &texto(p, "nome")?).map_err(erro).and_then(|_| ok()),
        "lancamento.trecho" => {
            let voz = p.get("voz").and_then(Value::as_str).map(String::from);
            let nome = if voz.is_none() { Some(prefixo(&texto(p, "nome")?, 120)) } else { None };
            lance.reatribuir(&texto(p, "id")?, &texto(p, "trecho")?, voz.as_deref(), nome.as_deref()).map_err(erro).and_then(|_| ok())
        }
        "historico" => {
            let busca = p.get("busca").and_then(Value::as_str).unwrap_or("");
            let limite = p.get("limite").and_then(Value::as_f64).unwrap_or(200.0).clamp(1.0, 1000.0) as usize;
            banco.historico(limite, busca).map_err(erro).and_then(json_de)
        }
        "estatisticas" => {
            let dias = p.get("dias").and_then(Value::as_f64).unwrap_or(30.0).clamp(7.0, 365.0) as u32;
            banco.estatisticas(dias).map_err(erro).and_then(json_de)
        }
        "dicionario.listar" => banco.termos().map_err(erro).and_then(json_de),
        "dicionario.salvar" => {
            let termo = p.get("termo").and_then(Value::as_str).ok_or("falta termo")?;
            let formas: Vec<String> = p.get("ouvidoComo").and_then(Value::as_array).map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default();
            let id = p.get("id").and_then(Value::as_f64).unwrap_or(0.0) as i64;
            banco.salvar_termo(id, termo, formas).map_err(erro).and_then(json_de)
        }
        "dicionario.apagar" => { banco.apagar_termo(inteiro(p, "id")?).map_err(erro)?; ok() }
        _ => Err(outro.into()),
    }
}

fn estado(casca: &Casca) -> Value {
    let c = &casca.controlador;
    let tecla = casca.prefs.atalho();
    let motores = &casca.integracao.motores;
    let (rec, detalhe) = motores.estado_reconhecedor();
    let (limpador, detalhe_limpador) = motores.estado_limpador();
    let mut modelo = motores.modelo_ia();
    modelo["aviso"] = json!(casca.prefs.ler("avisoModelo") == "1");
    json!({
        "fase": c.fase().id(),
        "maosLivres": c.maos_livres(),
        "reconhecedor": {"estado": rec, "detalhe": detalhe, "nome": "Parakeet"},
        "limpador": {"estado": limpador, "detalhe": detalhe_limpador, "ligado": casca.prefs.limpeza_ia(), "modelo": crate::motores_locais::modelos_do_catalogo()[0].trim_end_matches(".gguf")},
        "modeloIA": modelo,
        "permissoes": {"microfone": if audio::microfone_disponivel() { "concedida" } else { "negada" }, "acessibilidade": "concedida"},
        "atalho": {"tecla": tecla.id(), "nome": tecla.nome(), "simbolo": tecla.simbolo(), "instalado": *c.atalho_instalado.lock().unwrap()},
        "wisprAberto": false,
        "wisprDisponivel": false,
        "versao": casca.app.package_info().version.to_string(),
    })
}

fn ajustes(casca: &Casca) -> Value {
    let modelos = crate::motores_locais::modelos_instalados(&caminhos::modelos());
    json!({
        "valores": casca.prefs.todas(),
        "teclas": Tecla::TODAS.iter().map(|t| json!({"id": t.id(), "nome": t.nome()})).collect::<Vec<_>>(),
        "microfones": audio::microfones().into_iter().map(|(uid, nome)| json!({"uid": uid, "nome": nome})).collect::<Vec<_>>(),
        "modelos": modelos,
        "pastaDeModelos": caminhos::modelos().to_string_lossy(),
        "llamaInstalado": casca.integracao.motores.llama_instalado(),
    })
}

#[cfg(test)]
mod testes {
    use super::*;
    use std::collections::BTreeSet;

    /// Todo método que a interface chama tem que estar atendido aqui.
    fn chamados_pela_interface() -> BTreeSet<String> {
        let mut nomes = BTreeSet::new();
        let mut pilha = vec![PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../ui/src"))];
        while let Some(p) = pilha.pop() {
            for e in std::fs::read_dir(&p).unwrap().flatten() {
                let caminho = e.path();
                if caminho.is_dir() { pilha.push(caminho); continue; }
                if caminho.file_name().map(|n| n == "exemplo.ts").unwrap_or(false) { continue; }
                let texto = std::fs::read_to_string(&caminho).unwrap_or_default();
                for (marca, prefixo) in [("chamar(\"", ""), ("chamar<", ""), ("acaoLocal(\"", "lancamento."), ("useDados<", ""), ("useDados(\"", "")] {
                    for pedaco in texto.split(marca).skip(1) {
                        let resto = if marca.ends_with('<') { pedaco.split_once(">(\"").map(|(_, r)| r).unwrap_or("") } else { pedaco };
                        let nome: String = resto.chars().take_while(|c| c.is_alphanumeric() || *c == '.').collect();
                        if !nome.is_empty() && !nome.ends_with('.') { nomes.insert(format!("{prefixo}{nome}")); }
                    }
                }
            }
        }
        nomes
    }

    #[test]
    fn interface_so_chama_metodos_atendidos() {
        let atendidos: BTreeSet<&str> = METODOS.iter().map(|(m, _)| *m).collect();
        let chamados = chamados_pela_interface();
        assert!(chamados.len() >= 25, "varredura achou poucos métodos: {chamados:?}");
        let faltando: Vec<_> = chamados.iter().filter(|m| !atendidos.contains(m.as_str())).collect();
        assert!(faltando.is_empty(), "métodos chamados pela interface sem atendimento: {faltando:?}");
    }

    mod integrada {
        use std::{sync::{Arc, Mutex}, thread, time::{Duration, Instant}};

        use serde_json::{json, Value};

        use crate::{integracao::{dubles::{voz, MotoresDuble}, Integracao}, lancamento::{dubles::CapturaDuble, Lancamento}, ponte::{atender_dados, METODOS, Situacao}};

        /// A ordem dos campos é a ordem de soltura: a pasta temporária sai por último.
        struct Ponte { banco: Integracao, lance: Arc<Lancamento>, eventos: Arc<Mutex<Vec<String>>>, _tmp: Arc<tempfile::TempDir> }

        impl Drop for Ponte {
            fn drop(&mut self) { self.lance.desligar(Duration::from_secs(20)); self.lance.esperar_ocioso(Duration::from_secs(20)); }
        }

        impl Ponte {
            fn nova() -> Self { Self::na(Arc::new(tempfile::tempdir().unwrap())) }
            /// Outra ponte na mesma pasta: o app reaberto.
            fn na(tmp: Arc<tempfile::TempDir>) -> Self {
                let motores = MotoresDuble::novo();
                let banco = Integracao::abrir(&tmp.path().join("d.sqlite"), motores.clone()).unwrap();
                let eventos = Arc::new(Mutex::new(Vec::new()));
                let e = eventos.clone();
                let lance = Lancamento::new(&tmp.path().join("d.sqlite"), &tmp.path().join("Audio"), motores, Arc::new(CapturaDuble { segundos: 3.0 }),
                    Arc::new(|t: &str| t.to_string()), Arc::new(move |a, _| e.lock().unwrap().push(a.to_string()))).unwrap();
                Self { banco, lance, eventos, _tmp: tmp }
            }
            /// Chama como a interface: nome do método e parâmetros JSON; devolve o JSON lido da resposta em texto.
            fn chamar(&self, metodo: &str, params: Value) -> Result<Value, String> {
                let r = atender_dados(&self.banco, &self.lance, metodo, &params).unwrap_or_else(|| panic!("{metodo} não é método de dados"));
                r.map(|t| serde_json::from_str(&t).unwrap())
            }
            fn esperar(&self, o_que: &str, mut f: impl FnMut(&Self) -> bool) {
                let inicio = Instant::now();
                while !f(self) { assert!(inicio.elapsed() < Duration::from_secs(30), "esperando {o_que}"); thread::sleep(Duration::from_millis(50)); }
            }
        }

        #[test]
        fn nenhum_metodo_fica_sem_atendimento_real_salvo_wispr() {
            let fora: Vec<_> = METODOS.iter().filter(|(_, s)| *s == Situacao::Indisponivel).map(|(m, _)| *m).collect();
            assert_eq!(fora, ["wispr.importar"]);
            let fonte = include_str!("integracao.rs").to_owned() + include_str!("lancamento.rs") + include_str!("ponte.rs");
            assert!(!fonte.contains(&["Ainda não disponível", " nesta versão de teste"].concat()), "sobrou stub");
        }

        #[test]
        fn dicionario_historico_e_estatisticas_pela_ponte() {
            let p = Ponte::nova();
            let t = p.chamar("dicionario.salvar", json!({"termo": "Dita.se", "ouvidoComo": ["dita se"]})).unwrap();
            assert_eq!(p.chamar("dicionario.listar", json!({})).unwrap()[0]["termo"], "Dita.se");
            assert_eq!(p.chamar("dicionario.apagar", json!({"id": t["id"]})).unwrap(), true);
            assert_eq!(p.chamar("historico", json!({"busca": "", "limite": 50})).unwrap(), json!([]));
            let e = p.chamar("estatisticas", json!({"dias": 30})).unwrap();
            assert_eq!(e["porDia"].as_array().unwrap().len(), 30);
            assert!(p.chamar("dicionario.apagar", json!({})).is_err(), "sem id recusa");
        }

        #[test]
        fn arquivo_e_edicoes_pela_ponte() {
            let p = Ponte::nova();
            let estado = p.chamar("lancamento.estado", json!({})).unwrap();
            assert_eq!(estado["disponivel"], true);
            let arq = p._tmp.path().join("aula.wav");
            crate::audio::EscritorWav::criar(&arq).unwrap().gravar(&voz(3.0, 0.5)).unwrap();
            let id = p.lance.adicionar(&[arq]).unwrap().unwrap();
            p.esperar("arquivo pronto", |p| p.chamar("arquivo.listar", json!({})).unwrap()[0]["fase"] == "pronto");
            let doc = p.chamar("lancamento.documento", json!({"id": id})).unwrap();
            let trecho = doc["trechos"][0]["id"].clone();
            assert_eq!(p.chamar("lancamento.buscarArquivos", json!({"termo": "palavra1"})).unwrap(), json!([id]));
            assert_eq!(p.chamar("lancamento.titulo", json!({"id": id, "nome": "Aula 1"})).unwrap(), true);
            assert_eq!(p.chamar("falante.renomear", json!({"id": id, "voz": "offline-1", "nome": "Ana"})).unwrap(), true);
            assert_eq!(p.chamar("lancamento.trecho", json!({"id": id, "trecho": trecho, "nome": "Bia"})).unwrap(), true);
            let doc = p.chamar("lancamento.documento", json!({"id": id})).unwrap();
            assert_eq!(doc["nome"], "Aula 1");
            assert!(doc["vozes"].as_array().unwrap().iter().any(|v| v["nome"] == "Bia"), "{doc}");
            assert_eq!(p.chamar("lancamento.remover", json!({"id": id})).unwrap(), true);
            assert!(p.chamar("arquivo.listar", json!({})).unwrap().as_array().unwrap().is_empty());
            assert_eq!(p.chamar("lancamento.desfazer", json!({"id": id})).unwrap(), true);
            assert_eq!(p.chamar("arquivo.listar", json!({})).unwrap().as_array().unwrap().len(), 1);
            assert!(p.eventos.lock().unwrap().iter().any(|e| e == "ditado:progresso"));
        }

        /// Sequência que caía no CI Windows: arquivo e reunião pela mesma ponte, com `lancamento.estado`
        /// (consulta de microfone) chamado de threads diferentes que terminam.
        #[test]
        fn reuniao_depois_de_arquivo_na_mesma_ponte_com_estado_em_outras_threads() {
            let p = Arc::new(Ponte::nova());
            let arq = p._tmp.path().join("aula.wav");
            crate::audio::EscritorWav::criar(&arq).unwrap().gravar(&voz(2.0, 0.5)).unwrap();
            p.lance.adicionar(&[arq]).unwrap();
            let em_thread = |p: &Arc<Ponte>, metodo: &'static str, params: Value| { let p = p.clone(); thread::spawn(move || p.chamar(metodo, params)).join().unwrap() };
            p.esperar("arquivo pronto", |p| p.chamar("arquivo.listar", json!({})).unwrap()[0]["fase"] == "pronto");
            assert_eq!(em_thread(&p, "lancamento.estado", json!({})).unwrap()["disponivel"], true);
            em_thread(&p, "reuniao.iniciar", json!({"fonte": "macEMicrofone"})).unwrap();
            let id = em_thread(&p, "lancamento.estado", json!({})).unwrap()["reuniao"]["id"].as_str().unwrap().to_string();
            assert_eq!(em_thread(&p, "reuniao.pausar", json!({"id": id})).unwrap(), true);
            assert_eq!(em_thread(&p, "reuniao.pausar", json!({"id": id})).unwrap(), true);
            assert_eq!(em_thread(&p, "reuniao.encerrar", json!({"id": id})).unwrap(), true);
            p.esperar("reunião arquivada", |p| p.chamar("reuniao.listar", json!({})).unwrap().as_array().unwrap().iter().any(|r| r["id"] == json!(id)));
        }

        /// Ponte solta com a reunião gravando: as threads de captura e processamento terminam e
        /// soltam o serviço (antes ficavam vivas para sempre com a pasta aberta), e o app reaberto
        /// na mesma pasta encontra a reunião sem perder fala.
        #[test]
        fn ponte_solta_com_reuniao_em_andamento_encerra_threads_e_reabre() {
            let tmp = Arc::new(tempfile::tempdir().unwrap());
            let id;
            {
                let p = Ponte::na(tmp.clone());
                p.chamar("reuniao.iniciar", json!({"fonte": "mac"})).unwrap();
                id = p.chamar("lancamento.estado", json!({})).unwrap()["reuniao"]["id"].as_str().unwrap().to_string();
                thread::sleep(Duration::from_millis(300));
                let lance = p.lance.clone();
                drop(p);
                let inicio = Instant::now();
                while Arc::strong_count(&lance) > 1 { assert!(inicio.elapsed() < Duration::from_secs(10), "threads da reunião não soltaram o serviço"); thread::sleep(Duration::from_millis(20)); }
            }
            let p = Ponte::na(tmp);
            let doc = p.chamar("lancamento.documento", json!({"id": id})).unwrap();
            let recuperavel = p.chamar("lancamento.estado", json!({})).unwrap()["recuperaveis"].as_array().unwrap().iter().any(|r| r["id"] == json!(id));
            assert!(doc["fase"] == "pronta" || recuperavel, "reunião perdida: {doc}");
            assert!(p.chamar("reuniao.iniciar", json!({"fonte": "mac"})).is_ok(), "app reaberto inicia outra reunião");
        }

        #[test]
        fn reuniao_pela_ponte_ate_arquivada() {
            let p = Ponte::nova();
            assert!(p.chamar("lancamento.iniciar", json!({"fonte": "radio"})).is_err());
            p.chamar("reuniao.iniciar", json!({"fonte": "macEMicrofone"})).unwrap();
            let id = p.chamar("lancamento.estado", json!({})).unwrap()["reuniao"]["id"].as_str().unwrap().to_string();
            assert_eq!(p.chamar("reuniao.pausar", json!({"id": id})).unwrap(), true);
            assert_eq!(p.chamar("lancamento.pausar", json!({"id": id})).unwrap(), true);
            thread::sleep(Duration::from_millis(1200));
            assert_eq!(p.chamar("reuniao.encerrar", json!({"id": id})).unwrap(), true);
            p.esperar("reunião arquivada", |p| p.chamar("reuniao.listar", json!({})).unwrap().as_array().unwrap().iter().any(|r| r["id"] == json!(id)));
            assert!(p.chamar("lancamento.estado", json!({})).unwrap().get("reuniao").is_none());
            assert!(p.chamar("lancamento.resumir", json!({"id": id})).is_err(), "sem IA local, resumo recusa com motivo");
            assert_eq!(p.chamar("lancamento.cancelarResumo", json!({"id": id})).unwrap(), true);
        }
    }

    /// M1 e M3: atualização exige versão assinada; a pílula não tem a ponte; eventos iguais aos da interface.
    #[test]
    fn configuracao_de_seguranca() {
        let conf: Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(conf["plugins"]["updater"]["requireSignedVersion"], true);
        let pilula: Value = serde_json::from_str(include_str!("../capabilities/pilula.json")).unwrap();
        assert_eq!(pilula["windows"], serde_json::json!(["pilula"]));
        let permissoes: Vec<&str> = pilula["permissions"].as_array().unwrap().iter().map(|p| p.as_str().unwrap()).collect();
        assert!(!permissoes.contains(&"allow-ponte") && !permissoes.contains(&"core:default"), "{permissoes:?}");
        let principal: Value = serde_json::from_str(include_str!("../capabilities/principal.json")).unwrap();
        assert_eq!(principal["windows"], serde_json::json!(["principal"]));
        let ts = include_str!("../../../ui/src/lib/ponte.ts");
        for e in EVENTOS { assert!(ts.contains(&format!("\"{e}\"")), "{e} não está na lista da interface"); }
        assert!(!include_str!("pilula.rs").contains(".eval("), "pílula ainda avalia JavaScript");
        assert!(!include_str!("ponte.rs").contains(&["j.ev", "al("].concat()), "ponte ainda avalia JavaScript");
    }

    #[test]
    fn navegacao_so_para_a_interface_empacotada() {
        let ok = |u: &str| crate::navegacao_permitida(&tauri::Url::parse(u).unwrap());
        assert!(ok("http://tauri.localhost/index.html") && ok("tauri://localhost/pilula.html"));
        assert!(!ok("https://exemplo.com/") && !ok("http://tauri.localhost.exemplo.com/") && !ok("file:///C:/x.html"));
    }

    #[test]
    fn todos_os_metodos_tem_um_braco_no_atender() {
        let fonte = include_str!("ponte.rs");
        let corpo = &fonte[fonte.find("pub fn atender").unwrap()..fonte.find("fn estado(casca").unwrap()];
        for (m, _) in METODOS { assert!(corpo.contains(&format!("\"{m}\"")), "{m} sem braço em atender"); }
        let unicos: BTreeSet<_> = METODOS.iter().map(|(m, _)| m).collect();
        assert_eq!(unicos.len(), METODOS.len(), "método repetido em METODOS");
    }
}
