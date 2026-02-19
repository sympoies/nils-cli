use std::path::PathBuf;

fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn provider_runtime_source_does_not_import_codex_cli() {
    let adapter_path = crate_root().join("src").join("adapter.rs");
    let source = std::fs::read_to_string(&adapter_path).expect("read adapter source");

    assert!(
        !source.contains("codex_cli::"),
        "unexpected codex_cli import in {}",
        adapter_path.display()
    );
}

#[test]
fn provider_manifest_uses_codex_core_and_not_codex_cli() {
    let cargo_toml = crate_root().join("Cargo.toml");
    let manifest = std::fs::read_to_string(&cargo_toml).expect("read Cargo.toml");

    assert!(
        manifest.contains("codex-core"),
        "expected codex-core dependency in {}",
        cargo_toml.display()
    );
    assert!(
        !manifest.contains("codex-cli ="),
        "unexpected codex-cli dependency in {}",
        cargo_toml.display()
    );
}
