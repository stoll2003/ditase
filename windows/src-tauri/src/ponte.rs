//! Ponte entre a interface e o app (porta de `Ponte.swift`). A interface chama
//! `invoke("ponte", {metodo, params})` pelo adaptador de `ui/src/lib/ponte.ts` e recebe o mesmo
//! JSON em texto que o Mac devolve. Os avisos à interface usam o mesmo `CustomEvent` do Mac.
use std::path::PathBuf;

use serde_json::{json, Value};
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::DialogExt;

use crate::{audio, caminhos, decodificacao, nativo, preferencias::Tecla, Casca};

/// Situação de cada método do contrato no Windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Situacao {
    /// Implementado na casca, sem depender do núcleo.
    Pronto,
    /// A casca faz a parte nativa; o resultado depende do núcleo ou dos motores (ver `integracao`).
    DependeDoNucleo,
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
    ("historico", Situacao::DependeDoNucleo),
    ("estatisticas", Situacao::DependeDoNucleo),
    ("dicionario.listar", Situacao::DependeDoNucleo),
    ("dicionario.salvar", Situacao::DependeDoNucleo),
    ("dicionario.apagar", Situacao::DependeDoNucleo),
    ("ditado.apagar", Situacao::DependeDoNucleo),
    ("ditado.retranscrever", Situacao::DependeDoNucleo),
    ("wispr.importar", Situacao::DependeDoNucleo),
    ("modelo.baixar", Situacao::DependeDoNucleo),
    ("modelo.cancelar", Situacao::DependeDoNucleo),
    ("lancamento.estado", Situacao::DependeDoNucleo),
    ("lancamento.documento", Situacao::DependeDoNucleo),
    ("lancamento.buscarArquivos", Situacao::DependeDoNucleo),
    ("lancamento.recuperar", Situacao::DependeDoNucleo),
    ("lancamento.descartar", Situacao::DependeDoNucleo),
    ("lancamento.escolher", Situacao::DependeDoNucleo),
    ("lancamento.adicionar", Situacao::DependeDoNucleo),
    ("arquivo.transcrever", Situacao::DependeDoNucleo),
    ("lancamento.repetir", Situacao::DependeDoNucleo),
    ("arquivo.listar", Situacao::DependeDoNucleo),
    ("reuniao.listar", Situacao::DependeDoNucleo),
    ("arquivo.cancelar", Situacao::DependeDoNucleo),
    ("lancamento.remover", Situacao::DependeDoNucleo),
    ("lancamento.desfazer", Situacao::DependeDoNucleo),
    ("lancamento.cancelarFila", Situacao::DependeDoNucleo),
    ("lancamento.iniciar", Situacao::DependeDoNucleo),
    ("reuniao.iniciar", Situacao::DependeDoNucleo),
    ("lancamento.pausar", Situacao::DependeDoNucleo),
    ("reuniao.pausar", Situacao::DependeDoNucleo),
    ("lancamento.encerrar", Situacao::DependeDoNucleo),
    ("reuniao.encerrar", Situacao::DependeDoNucleo),
    ("lancamento.resumir", Situacao::DependeDoNucleo),
    ("lancamento.cancelarResumo", Situacao::DependeDoNucleo),
    ("lancamento.titulo", Situacao::DependeDoNucleo),
    ("lancamento.voz", Situacao::DependeDoNucleo),
    ("falante.renomear", Situacao::DependeDoNucleo),
    ("lancamento.trecho", Situacao::DependeDoNucleo),
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
pub fn emitir(app: &AppHandle, assunto: &str) {
    if let Some(j) = app.get_webview_window("principal") {
        let _ = j.eval(&format!("window.dispatchEvent(new CustomEvent('ditado', {{ detail: {} }}))", json!(assunto)));
    }
}

/// Eventos com dados (`ditado:progresso`, `ditado:trechos`, `ditado:resultado`, `ditado:arquivos-soltos`).
pub fn emitir_evento(app: &AppHandle, evento: &str, detalhe: &Value) {
    if let Some(j) = app.get_webview_window("principal") {
        let _ = j.eval(&format!("window.dispatchEvent(new CustomEvent({}, {{ detail: {} }}))", json!(evento), detalhe));
    }
}

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
    match metodo {
        // MARK: Arquivos e reuniões
        "lancamento.estado" => json_de(lance.estado(audio::microfone_disponivel())),
        "lancamento.documento" => lance.documento(&texto(p, "id")?).map_err(erro).and_then(json_de),
        "lancamento.buscarArquivos" => {
            let termo = p.get("termo").and_then(Value::as_str).unwrap_or("").trim();
            if termo.is_empty() { Ok("[]".into()) } else { lance.procurar_arquivos(termo).map_err(erro).and_then(json_de) }
        }
        "lancamento.recuperar" => lance.recuperar_reuniao(&texto(p, "id")?).map_err(erro).and_then(|_| ok()),
        "lancamento.descartar" => lance.descartar_reuniao(&texto(p, "id")?).map_err(erro).and_then(|_| ok()),
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
        "arquivo.listar" => json_de(lance.estado(true)["arquivos"].clone()),
        "reuniao.listar" => json_de(lance.estado(true)["anteriores"].clone()),
        "arquivo.cancelar" => { lance.cancelar(&texto(p, "id")?); ok() }
        "lancamento.remover" => lance.remover(&texto(p, "id")?).map_err(erro).and_then(|_| ok()),
        "lancamento.desfazer" => lance.desfazer(&texto(p, "id")?).map_err(erro).and_then(|_| ok()),
        "lancamento.cancelarFila" => { lance.cancelar_fila(); ok() }
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
        "lancamento.permissao" => {
            // Áudio do sistema não pede permissão no Windows; o microfone abre a tela de Privacidade.
            match texto(p, "qual")?.as_str() {
                "audioSistema" => ok(),
                "microfone" => { let _ = nativo::sistema::abrir("ms-settings:privacy-microphone"); ok() }
                _ => Err("permissão desconhecida".into()),
            }
        }
        "lancamento.titulo" => lance.alterar_titulo(&texto(p, "id")?, &prefixo(&texto(p, "nome")?, 160)).map_err(erro).and_then(|_| ok()),
        "falante.renomear" | "lancamento.voz" => lance.renomear_voz(&texto(p, "id")?, &texto(p, "voz")?, &texto(p, "nome")?).map_err(erro).and_then(|_| ok()),
        "lancamento.trecho" => {
            let voz = p.get("voz").and_then(Value::as_str).map(String::from);
            let nome = if voz.is_none() { Some(prefixo(&texto(p, "nome")?, 120)) } else { None };
            lance.reatribuir(&texto(p, "id")?, &texto(p, "trecho")?, voz.as_deref(), nome.as_deref()).map_err(erro).and_then(|_| ok())
        }

        // MARK: Ditado
        "estado" => json_de(estado(casca)),
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
        "ajustes.ler" => json_de(ajustes(casca)),
        "ajustes.definir" => {
            let (chave, valor) = match (p.get("chave").and_then(Value::as_str), p.get("valor").and_then(Value::as_str)) {
                (Some(k), Some(v)) => (k, v),
                _ => return Err("falta chave ou valor".into()),
            };
            if chave == "atalho" && Tecla::de_id(valor).is_none() { return Err("tecla desconhecida".into()); }
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
        "copiar" => { nativo::colagem::copiar(&p.get("texto").and_then(Value::as_str).ok_or("falta texto")?.to_string())?; ok() }
        "gravacao.alternar" => { c.alternar(); ok() }
        "wispr.importar" => {
            let resumo = banco.importar_wispr().map_err(erro)?;
            emitir(app, "historico");
            emitir(app, "dicionario");
            json_de(resumo)
        }
        "modelo.baixar" => { banco.baixar_modelo().map_err(erro)?; ok() }
        "modelo.cancelar" => { banco.cancelar_modelo().map_err(erro)?; ok() }
        "abrir.pastaDeModelos" => { nativo::sistema::abrir(&caminhos::modelos().to_string_lossy()).map_err(erro)?; ok() }
        "relatorio.copiar" => {
            let texto = crate::relatorio::montar(casca);
            nativo::colagem::copiar(&texto)?;
            json_de(texto)
        }
        _ => Err(format!("método desconhecido: {metodo}")),
    }
}

fn estado(casca: &Casca) -> Value {
    let c = &casca.controlador;
    let tecla = casca.prefs.atalho();
    let (rec, detalhe) = casca.integracao.reconhecedor();
    let mut modelo = casca.integracao.modelo_ia();
    modelo["aviso"] = json!(casca.prefs.ler("avisoModelo") == "1");
    json!({
        "fase": c.fase().id(),
        "maosLivres": c.maos_livres(),
        "reconhecedor": {"estado": rec, "detalhe": detalhe, "nome": "Parakeet"},
        "limpador": {"estado": "desligado", "detalhe": "", "ligado": casca.prefs.limpeza_ia(), "modelo": casca.prefs.modelo().trim_end_matches(".gguf")},
        "modeloIA": modelo,
        "permissoes": {"microfone": if audio::microfone_disponivel() { "concedida" } else { "negada" }, "acessibilidade": "concedida"},
        "atalho": {"tecla": tecla.id(), "nome": tecla.nome(), "simbolo": tecla.simbolo(), "instalado": *c.atalho_instalado.lock().unwrap()},
        "wisprAberto": false,
        "wisprDisponivel": false,
        "versao": casca.app.package_info().version.to_string(),
    })
}

fn ajustes(casca: &Casca) -> Value {
    let mut modelos: Vec<String> = std::fs::read_dir(caminhos::modelos()).map(|l| l.flatten().filter_map(|e| e.file_name().into_string().ok()).filter(|n| n.ends_with(".gguf")).collect()).unwrap_or_default();
    modelos.sort();
    json!({
        "valores": casca.prefs.todas(),
        "teclas": Tecla::TODAS.iter().map(|t| json!({"id": t.id(), "nome": t.nome()})).collect::<Vec<_>>(),
        "microfones": audio::microfones().into_iter().map(|(uid, nome)| json!({"uid": uid, "nome": nome})).collect::<Vec<_>>(),
        "modelos": modelos,
        "pastaDeModelos": caminhos::modelos().to_string_lossy(),
        "llamaInstalado": caminhos::llama_server().is_file(),
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

    #[test]
    fn todos_os_metodos_tem_um_braco_no_atender() {
        let fonte = include_str!("ponte.rs");
        let corpo = &fonte[fonte.find("pub fn atender").unwrap()..fonte.find("fn estado(casca").unwrap()];
        for (m, _) in METODOS { assert!(corpo.contains(&format!("\"{m}\"")), "{m} sem braço em atender"); }
        let unicos: BTreeSet<_> = METODOS.iter().map(|(m, _)| m).collect();
        assert_eq!(unicos.len(), METODOS.len(), "método repetido em METODOS");
    }
}
