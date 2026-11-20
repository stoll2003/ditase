pub fn tempdir() -> tempfile::TempDir {
    let raiz = std::env::temp_dir();
    // No macOS o temporário do sistema pode vir como /var, um alias de /private/var.
    #[cfg(unix)]
    let raiz = raiz.canonicalize().unwrap();
    tempfile::tempdir_in(raiz).unwrap()
}
