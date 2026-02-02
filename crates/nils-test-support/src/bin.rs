use std::path::PathBuf;

/// Resolve a workspace binary path for tests.
///
/// Cargo exposes test binaries via `CARGO_BIN_EXE_<name>`. This helper checks the
/// name as-is and also tries swapping `-` and `_` to match how Cargo exports
/// environment variables for hyphenated crate names.
///
/// When no env var is present, it falls back to `target/<profile>/<name>` based
/// on the current test executable location.
pub fn resolve(bin_name: &str) -> PathBuf {
    let candidates = env_names(bin_name);
    for candidate in candidates {
        if let Ok(bin) = std::env::var(&candidate) {
            return PathBuf::from(bin);
        }
    }

    let exe = std::env::current_exe().expect("current exe");
    let target_dir = exe.parent().and_then(|p| p.parent()).expect("target dir");
    let bin_file = format!("{bin_name}{}", std::env::consts::EXE_SUFFIX);
    let bin = target_dir.join(bin_file);
    if bin.exists() {
        return bin;
    }

    panic!("{bin_name} binary path: NotPresent");
}

fn env_names(bin_name: &str) -> Vec<String> {
    let mut names = Vec::new();
    names.push(format!("CARGO_BIN_EXE_{bin_name}"));

    if bin_name.contains('-') {
        names.push(format!("CARGO_BIN_EXE_{}", bin_name.replace('-', "_")));
    }
    if bin_name.contains('_') {
        names.push(format!("CARGO_BIN_EXE_{}", bin_name.replace('_', "-")));
    }

    names
}

#[cfg(test)]
mod tests {
    use super::env_names;

    #[test]
    fn env_names_includes_variants() {
        let names = env_names("api-test");
        assert_eq!(
            names,
            vec![
                "CARGO_BIN_EXE_api-test".to_string(),
                "CARGO_BIN_EXE_api_test".to_string(),
            ]
        );

        let names = env_names("api_test");
        assert_eq!(
            names,
            vec![
                "CARGO_BIN_EXE_api_test".to_string(),
                "CARGO_BIN_EXE_api-test".to_string(),
            ]
        );
    }
}
