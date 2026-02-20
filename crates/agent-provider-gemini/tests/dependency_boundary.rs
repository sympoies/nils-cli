use std::path::{Path, PathBuf};

fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn collect_rust_files(dir: &Path, acc: &mut Vec<PathBuf>) {
    let entries = std::fs::read_dir(dir).expect("read source directory");
    for entry in entries {
        let path = entry.expect("read source entry").path();
        if path.is_dir() {
            collect_rust_files(&path, acc);
            continue;
        }

        if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            acc.push(path);
        }
    }
}

#[test]
fn provider_runtime_source_does_not_import_gemini_cli() {
    let src_dir = crate_root().join("src");
    let mut rust_files = Vec::new();
    collect_rust_files(&src_dir, &mut rust_files);
    assert!(
        !rust_files.is_empty(),
        "expected at least one rust source file under {}",
        src_dir.display()
    );

    for source_path in rust_files {
        let source = std::fs::read_to_string(&source_path).expect("read rust source");
        assert!(
            !source.contains("gemini_cli::"),
            "unexpected gemini_cli import in {}",
            source_path.display()
        );
    }
}

#[test]
fn provider_manifest_does_not_depend_on_gemini_cli_package() {
    let cargo_toml = crate_root().join("Cargo.toml");
    let manifest = std::fs::read_to_string(&cargo_toml).expect("read Cargo.toml");

    for (line_no, line) in manifest.lines().enumerate() {
        let trimmed = line.trim_start();
        assert!(
            !(trimmed.starts_with("gemini-cli") && trimmed.contains('=')),
            "unexpected gemini-cli dependency on line {} in {}",
            line_no + 1,
            cargo_toml.display()
        );
    }

    assert!(
        !manifest.contains("package = \"nils-gemini-cli\""),
        "unexpected dependency alias pointing to nils-gemini-cli in {}",
        cargo_toml.display()
    );

    assert!(
        manifest.contains("gemini-core"),
        "expected gemini-core dependency in {}",
        cargo_toml.display()
    );
}
