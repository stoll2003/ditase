//! "Copiar relatório de teste": o que o Vinicius cola no chat depois de testar no PC.
//! Só versão, Windows, situação das peças e as últimas linhas do registro. O registro nunca
//! recebe texto ditado nem título de janela; mesmo assim, linhas com aspas são cortadas.
use crate::{audio, caminhos, Casca};

pub fn montar(casca: &Casca) -> String {
    let mut r = String::new();
    let versao = casca.app.package_info().version.to_string();
    r.push_str(&format!("Relatório de teste do Dita.se para Windows\nVersão: {versao}\nWindows: {}\n", crate::nativo::sistema::versao()));
    r.push_str(&format!("Atalho: {} (gancho {})\n", casca.prefs.atalho().nome(), if *casca.controlador.atalho_instalado.lock().unwrap() { "instalado" } else { "NÃO instalado" }));
    let microfones = audio::microfones();
    r.push_str(&format!("Microfones: {} encontrado(s); escolhido: {}\n", microfones.len(), if casca.prefs.microfone().is_empty() { "padrão" } else { "outro" }));
    r.push_str(&format!("Fase do ditado: {}\n", casca.controlador.fase().id()));
    let fora = crate::ponte::METODOS.iter().filter(|(_, s)| *s == crate::ponte::Situacao::Indisponivel).count();
    r.push_str(&format!("Ponte: {} métodos, {fora} sem equivalente no Windows (importar do Wispr Flow)\n", crate::ponte::METODOS.len()));
    let motores = &casca.integracao.motores;
    let (fala, _) = motores.estado_reconhecedor();
    let (limpador, _) = motores.estado_limpador();
    r.push_str(&format!("Modelo de fala: {fala}; IA local: {limpador} (limpeza {}), modelo {}\n",
        if casca.prefs.limpeza_ia() { "ligada" } else { "desligada" },
        motores.modelo_ia()["estado"].as_str().unwrap_or("?")));
    r.push_str("\nÚltimas linhas do registro:\n");
    for linha in caminhos::ultimas_linhas(60) {
        // Defesa extra: nada entre aspas sai no relatório.
        let limpa = linha_aprovada(&linha);
        r.push_str(&limpa);
        r.push('\n');
    }
    r
}

/// Começos de linha do registro que podem ir para o relatório (B1). Qualquer outra vira um
/// marcador, sem o conteúdo. Mesmo nas aprovadas, nada entre aspas sai e caminhos perdem a conta.
pub const LINHAS_APROVADAS: &[&str] = &[
    "abrindo", "saída", "atalho", "áudio", "ditado ", "ditado recusado", "reunião ", "arquivo ", "abertura das reuniões",
    "modelos", "modelo de IA", "reconhecedor", "falantes", "limpeza por IA", "colagem", "atualização", "recuperação",
    "preferências", "teste", "abertura", "dados migrados", "migração", "pílula", "iniciar com o Windows", "microfone", "arraste",
];

pub fn linha_aprovada(linha: &str) -> String {
    let (hora, resto) = linha.split_once(' ').unwrap_or(("", linha));
    if !LINHAS_APROVADAS.iter().any(|p| resto.starts_with(p)) { return format!("{hora} [linha fora da lista aprovada]"); }
    let resto = if resto.contains('"') { format!("{} [trecho entre aspas omitido]", resto.split('"').next().unwrap_or("")) } else { resto.to_string() };
    format!("{hora} {}", anonimizar(&resto))
}

/// Tira o nome da conta dos caminhos (no Windows ele costuma ser o nome da pessoa): as pastas do
/// perfil viram `%USERPROFILE%` e afins, e qualquer `\Users\<conta>\` restante vira `<conta>`.
pub fn anonimizar(linha: &str) -> String {
    let mut s = linha.to_string();
    for var in ["LOCALAPPDATA", "APPDATA", "TEMP", "TMP", "USERPROFILE", "HOME"] {
        if let Some(v) = std::env::var_os(var) {
            let v = v.to_string_lossy().trim_end_matches(['\\', '/']).to_string();
            if v.len() > 3 { s = s.replace(&v, &format!("%{var}%")); }
        }
    }
    let mut saida = String::with_capacity(s.len());
    let mut resto = s.as_str();
    loop {
        // Minúsculas só ASCII: os índices continuam valendo no texto original.
        let minusculo = resto.to_ascii_lowercase();
        let achado = ["\\users\\", "/users/", "/home/"].iter().filter_map(|m| minusculo.find(m).map(|i| (i, m.len()))).min();
        let Some((i, n)) = achado else { saida.push_str(resto); break };
        saida.push_str(&resto[..i + n]);
        let depois = &resto[i + n..];
        // A conta pode ter espaço ("João Silva"): só um separador de caminho encerra o nome.
        let fim = depois.find(['\\', '/', ';']).unwrap_or(depois.len());
        if fim > 0 { saida.push_str("<conta>"); }
        resto = &depois[fim..];
    }
    saida
}

#[cfg(test)]
mod testes {
    use super::anonimizar;

    #[test]
    fn so_linhas_aprovadas_vao_para_o_relatorio() {
        use super::linha_aprovada;
        assert_eq!(linha_aprovada("1.0 ditado 7: 12 palavras, 3.0 s"), "1.0 ditado 7: 12 palavras, 3.0 s");
        assert_eq!(linha_aprovada("2.0 texto qualquer de outra origem"), "2.0 [linha fora da lista aprovada]");
        assert_eq!(linha_aprovada(r#"3.0 arquivo x: falhou ("C:\Users\ana\a.mp3")"#), "3.0 arquivo x: falhou ( [trecho entre aspas omitido]");
    }

    #[test]
    fn caminhos_do_perfil_perdem_o_nome_da_conta() {
        assert_eq!(anonimizar(r"modelo: falta C:\Users\João Silva\AppData\Local\Dita.se\Models\x.onnx"), r"modelo: falta C:\Users\<conta>\AppData\Local\Dita.se\Models\x.onnx");
        assert_eq!(anonimizar("wav: /Users/maria/Library/x.wav e /home/ana/y"), "wav: /Users/<conta>/Library/x.wav e /home/<conta>/y");
        assert_eq!(anonimizar(r"D:\USERS\pedro\a"), r"D:\USERS\<conta>\a");
        assert_eq!(anonimizar("reconhecedor carregado em 3.1 s"), "reconhecedor carregado em 3.1 s");
        if let Some(casa) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
            let linha = format!("falha em {}{}x", casa.to_string_lossy(), std::path::MAIN_SEPARATOR);
            assert!(!anonimizar(&linha).contains(&*casa.to_string_lossy()), "{}", anonimizar(&linha));
        }
    }
}
