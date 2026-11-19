//! Monta `windows/app-dist` (interface compilada + página da pílula) antes do `generate_context!`,
//! e lê a chave pública do canal de teste de `windows/updater.pub`, quando existir.
use std::{env, fs, path::Path};

fn copiar(de: &Path, para: &Path) {
    fs::create_dir_all(para).expect("criar app-dist");
    for entrada in fs::read_dir(de).expect("ler ui/dist") {
        let entrada = entrada.unwrap();
        let destino = para.join(entrada.file_name());
        if entrada.file_type().unwrap().is_dir() { copiar(&entrada.path(), &destino) } else { fs::copy(entrada.path(), destino).unwrap(); }
    }
}

fn main() {
    let raiz = Path::new(env!("CARGO_MANIFEST_DIR"));
    let ui = raiz.join("../../ui/dist");
    let pilula = raiz.join("../pilula/pilula.html");
    let saida = raiz.join("../app-dist");
    let _ = fs::remove_dir_all(&saida);
    if ui.join("index.html").exists() {
        copiar(&ui, &saida);
    } else {
        // `cargo check` antes do `npm run build`: página provisória, o app real exige a interface compilada.
        fs::create_dir_all(&saida).unwrap();
        fs::write(saida.join("index.html"), "<!doctype html><meta charset=utf-8><title>Dita.se</title><p>Interface não compilada: rode npm run build em ui/.</p>").unwrap();
        println!("cargo:warning=ui/dist ausente: app-dist usa página provisória");
    }
    fs::copy(&pilula, saida.join("pilula.html")).expect("windows/pilula/pilula.html");
    println!("cargo:rerun-if-changed=../../ui/dist");
    println!("cargo:rerun-if-changed=../pilula/pilula.html");

    let chave = raiz.join("../updater.pub");
    println!("cargo:rerun-if-changed=../updater.pub");
    let pub_key = fs::read_to_string(&chave).map(|s| s.trim().to_string()).unwrap_or_default();
    if pub_key.is_empty() { println!("cargo:warning=windows/updater.pub ausente: atualização do canal de teste desligada"); }
    println!("cargo:rustc-env=DITASE_UPDATER_PUB={pub_key}");

    // Manifesto próprio: o padrão do Tauri mais `activeCodePage` UTF-8 (pedido dos motores).
    println!("cargo:rerun-if-changed=app.manifest");
    let windows = tauri_build::WindowsAttributes::new().app_manifest(include_str!("app.manifest"));
    tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows)).expect("tauri-build");
}
