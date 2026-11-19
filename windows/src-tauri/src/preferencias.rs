//! Ajustes do usuário (porta de `Preferencias.swift`). No Windows ficam em JSON
//! (`%APPDATA%\Dita.se\preferencias.json`), não na tabela `config` do banco, para a casca
//! funcionar antes do núcleo estar integrado. Mesmas chaves e padrões do Mac, com duas
//! diferenças: atalho padrão Ctrl direito (o Alt direito é AltGr no ABNT2) e `iniciarComWindows`.
use std::{collections::BTreeMap, fs, path::PathBuf, sync::Mutex};

use serde_json::{json, Value};

/// Teclas aceitas como atalho. Só uma é observada pelo gancho; nenhuma outra tecla é guardada.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tecla { ControleDireito, ShiftDireito, AltDireito }

impl Tecla {
    /// Ids combinados com a interface; ela mostra a lista que o app mandar e avisa sobre o AltGr.
    pub const TODAS: [Tecla; 3] = [Tecla::ControleDireito, Tecla::ShiftDireito, Tecla::AltDireito];
    pub fn id(self) -> &'static str {
        match self { Tecla::ControleDireito => "controleDireito", Tecla::ShiftDireito => "shiftDireito", Tecla::AltDireito => "altDireito" }
    }
    pub fn nome(self) -> &'static str {
        match self { Tecla::ControleDireito => "Ctrl direito", Tecla::ShiftDireito => "Shift direito", Tecla::AltDireito => "Alt direito" }
    }
    pub fn simbolo(self) -> &'static str {
        match self { Tecla::ControleDireito => "Ctrl dir.", Tecla::ShiftDireito => "Shift dir.", Tecla::AltDireito => "Alt dir." }
    }
    /// Código virtual do Windows (VK_RCONTROL, VK_RSHIFT, VK_RMENU).
    pub fn vk(self) -> u32 {
        match self { Tecla::ControleDireito => 0xA3, Tecla::ShiftDireito => 0xA1, Tecla::AltDireito => 0xA5 }
    }
    pub fn de_id(id: &str) -> Option<Tecla> { Self::TODAS.into_iter().find(|t| t.id() == id) }
}

pub const PADROES: &[(&str, &str)] = &[
    ("atalho", "controleDireito"),
    ("limpezaIA", "1"),
    ("modelo", "Qwen3.5-2B-Q4_K_M.gguf"),
    ("sons", "1"),
    ("limiteMinutos", "120"),
    ("pararEmSilencioMinutos", "5"),
    ("guardarAudioDias", "7"),
    ("microfone", ""),
    ("pilulaSempreVisivel", "1"),
    ("espacoNoFim", "1"),
    ("avisoModelo", "1"),
    ("iniciarComWindows", "0"),
];

pub struct Preferencias { arquivo: PathBuf, valores: Mutex<BTreeMap<String, String>> }

impl Preferencias {
    pub fn abrir(arquivo: PathBuf) -> Self {
        let valores = fs::read_to_string(&arquivo).ok()
            .and_then(|t| serde_json::from_str::<BTreeMap<String, String>>(&t).ok())
            .unwrap_or_default();
        Self { arquivo, valores: Mutex::new(valores) }
    }

    pub fn ler(&self, chave: &str) -> String {
        if let Some(v) = self.valores.lock().unwrap().get(chave) { return v.clone(); }
        PADROES.iter().find(|(k, _)| *k == chave).map(|(_, v)| v.to_string()).unwrap_or_default()
    }

    /// Chave desconhecida é ignorada, como no Mac. Devolve se mudou.
    pub fn definir(&self, chave: &str, valor: &str) -> bool {
        if !PADROES.iter().any(|(k, _)| *k == chave) { return false; }
        let mut v = self.valores.lock().unwrap();
        if v.get(chave).map(String::as_str) == Some(valor) { return false; }
        v.insert(chave.into(), valor.into());
        let texto = serde_json::to_string_pretty(&*v).unwrap_or_default();
        // Grava em temporário e troca: queda no meio não corrompe o arquivo.
        let tmp = self.arquivo.with_extension("json.tmp");
        if fs::write(&tmp, texto).and_then(|_| fs::rename(&tmp, &self.arquivo)).is_err() {
            crate::caminhos::nota("preferências: não gravou o arquivo");
        }
        true
    }

    pub fn todas(&self) -> Value {
        let mut m = serde_json::Map::new();
        for (k, _) in PADROES { m.insert(k.to_string(), json!(self.ler(k))); }
        Value::Object(m)
    }

    pub fn atalho(&self) -> Tecla { Tecla::de_id(&self.ler("atalho")).unwrap_or(Tecla::ControleDireito) }
    pub fn limpeza_ia(&self) -> bool { self.ler("limpezaIA") == "1" }
    pub fn modelo(&self) -> String { self.ler("modelo") }
    pub fn sons(&self) -> bool { self.ler("sons") == "1" }
    pub fn limite_minutos(&self) -> f64 { self.ler("limiteMinutos").parse::<f64>().unwrap_or(120.0).max(1.0) }
    pub fn parar_em_silencio_minutos(&self) -> f64 { self.ler("pararEmSilencioMinutos").parse::<f64>().unwrap_or(5.0).max(0.0) }
    pub fn guardar_audio_dias(&self) -> u32 { self.ler("guardarAudioDias").parse::<u32>().unwrap_or(7) }
    pub fn microfone(&self) -> String { self.ler("microfone") }
    pub fn pilula_sempre_visivel(&self) -> bool { self.ler("pilulaSempreVisivel") == "1" }
    pub fn espaco_no_fim(&self) -> bool { self.ler("espacoNoFim") == "1" }
    pub fn iniciar_com_windows(&self) -> bool { self.ler("iniciarComWindows") == "1" }
}

#[cfg(test)]
mod testes {
    use super::*;

    fn temporario(nome: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("ditase-prefs-{nome}-{}.json", std::process::id()));
        let _ = fs::remove_file(&p);
        p
    }

    #[test]
    fn padroes_e_chave_desconhecida() {
        let p = Preferencias::abrir(temporario("a"));
        assert_eq!(p.atalho(), Tecla::ControleDireito);
        assert!(!p.definir("inventada", "1"));
        assert_eq!(p.ler("inventada"), "");
        assert_eq!(p.todas().as_object().unwrap().len(), PADROES.len());
    }

    #[test]
    fn grava_e_reabre() {
        let arq = temporario("b");
        let p = Preferencias::abrir(arq.clone());
        assert!(p.definir("atalho", "shiftDireito"));
        assert!(!p.definir("atalho", "shiftDireito"));
        assert_eq!(Preferencias::abrir(arq).atalho(), Tecla::ShiftDireito);
    }
}
