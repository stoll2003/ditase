//! "Copiar relatório de teste": o que o Vinicius cola no chat depois de testar no PC.
//! Só versão, Windows, situação das peças e as últimas linhas do registro. O registro nunca
//! recebe texto ditado nem título de janela; mesmo assim, linhas com aspas são cortadas.
use crate::{audio, caminhos, integracao::PENDENCIAS, Casca};

pub fn montar(casca: &Casca) -> String {
    let mut r = String::new();
    let versao = casca.app.package_info().version.to_string();
    r.push_str(&format!("Relatório de teste do Dita.se para Windows\nVersão: {versao}\nWindows: {}\n", crate::nativo::sistema::versao()));
    r.push_str(&format!("Atalho: {} (gancho {})\n", casca.prefs.atalho().nome(), if *casca.controlador.atalho_instalado.lock().unwrap() { "instalado" } else { "NÃO instalado" }));
    let microfones = audio::microfones();
    r.push_str(&format!("Microfones: {} encontrado(s); escolhido: {}\n", microfones.len(), if casca.prefs.microfone().is_empty() { "padrão" } else { "outro" }));
    r.push_str(&format!("Fase do ditado: {}\n", casca.controlador.fase().id()));
    let prontos = crate::ponte::METODOS.iter().filter(|(_, s)| *s == crate::ponte::Situacao::Pronto).count();
    r.push_str(&format!("Ponte: {} métodos, {prontos} prontos na casca, {} dependem do núcleo\n", crate::ponte::METODOS.len(), crate::ponte::METODOS.len() - prontos));
    r.push_str("\nAinda dependem do núcleo/motores:\n");
    for (_, efeito) in PENDENCIAS { r.push_str(&format!("- {efeito}\n")); }
    r.push_str("\nÚltimas linhas do registro:\n");
    for linha in caminhos::ultimas_linhas(60) {
        // Defesa extra: nada entre aspas sai no relatório.
        let limpa = if linha.contains('"') { format!("{} [trecho entre aspas omitido]", linha.split('"').next().unwrap_or("")) } else { linha };
        r.push_str(&limpa);
        r.push('\n');
    }
    r
}
