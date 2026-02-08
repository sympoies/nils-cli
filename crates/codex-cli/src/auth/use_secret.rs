use anyhow::Result;
use std::path::{Path, PathBuf};

use crate::auth;
use crate::fs;
use crate::paths;

pub fn run(target: &str) -> Result<i32> {
    if target.is_empty() {
        eprintln!("codex-use: usage: codex-use <name|name.json|email>");
        return Ok(64);
    }

    if target.contains('/') || target.contains("..") {
        eprintln!("codex-use: invalid secret name: {target}");
        return Ok(64);
    }

    let secret_dir = match paths::resolve_secret_dir() {
        Some(dir) => dir,
        None => {
            eprintln!("codex-use: secret not found: {target}");
            return Ok(1);
        }
    };

    let is_email = target.contains('@');
    let mut secret_name = target.to_string();
    if !secret_name.ends_with(".json") && !is_email {
        secret_name.push_str(".json");
    }

    if secret_dir.join(&secret_name).is_file() {
        return apply_secret(&secret_dir, &secret_name);
    }

    match resolve_by_email(&secret_dir, target) {
        ResolveResult::Exact(name) => apply_secret(&secret_dir, &name),
        ResolveResult::Ambiguous { candidates } => {
            eprintln!("codex-use: identifier matches multiple secrets: {target}");
            eprintln!("codex-use: candidates: {}", candidates.join(", "));
            Ok(2)
        }
        ResolveResult::NotFound => {
            eprintln!("codex-use: secret not found: {target}");
            Ok(1)
        }
    }
}

fn apply_secret(secret_dir: &Path, secret_name: &str) -> Result<i32> {
    let source_file = secret_dir.join(secret_name);
    if !source_file.is_file() {
        eprintln!("codex: secret file {secret_name} not found");
        return Ok(1);
    }

    let auth_file = match paths::resolve_auth_file() {
        Some(path) => path,
        None => return Ok(1),
    };

    if auth_file.is_file() {
        let sync_result = crate::auth::sync::run()?;
        if sync_result != 0 {
            eprintln!("codex: failed to sync current auth before switching secrets");
            return Ok(1);
        }
    }

    let contents = std::fs::read(&source_file)?;
    fs::write_atomic(&auth_file, &contents, fs::SECRET_FILE_MODE)?;

    let iso = auth::last_refresh_from_auth_file(&auth_file).unwrap_or(None);
    let timestamp_path = secret_timestamp_path(&auth_file)?;
    fs::write_timestamp(&timestamp_path, iso.as_deref())?;

    println!("codex: applied {secret_name} to {}", auth_file.display());
    Ok(0)
}

fn resolve_by_email(secret_dir: &Path, target: &str) -> ResolveResult {
    let query = target.to_lowercase();
    let want_full = target.contains('@');

    let mut matches = Vec::new();
    if let Ok(entries) = std::fs::read_dir(secret_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let email = match auth::email_from_auth_file(&path) {
                Ok(Some(value)) => value,
                _ => continue,
            };
            let email_lower = email.to_lowercase();
            if want_full {
                if email_lower == query {
                    matches.push(file_name(&path));
                }
            } else if let Some(local_part) = email_lower.split('@').next() {
                if local_part == query {
                    matches.push(file_name(&path));
                }
            }
        }
    }

    if matches.len() == 1 {
        ResolveResult::Exact(matches.remove(0))
    } else if matches.is_empty() {
        ResolveResult::NotFound
    } else {
        ResolveResult::Ambiguous {
            candidates: matches,
        }
    }
}

fn secret_timestamp_path(target_file: &Path) -> Result<PathBuf> {
    let cache_dir = paths::resolve_secret_cache_dir()
        .ok_or_else(|| anyhow::anyhow!("CODEX_SECRET_CACHE_DIR not resolved"))?;
    let name = target_file
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("auth.json");
    Ok(cache_dir.join(format!("{name}.timestamp")))
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_string()
}

#[derive(Debug)]
enum ResolveResult {
    Exact(String),
    Ambiguous { candidates: Vec<String> },
    NotFound,
}

#[cfg(test)]
mod tests {
    use super::{file_name, resolve_by_email, secret_timestamp_path, ResolveResult};
    use pretty_assertions::assert_eq;
    use std::path::Path;

    const HEADER: &str = "eyJhbGciOiJub25lIiwidHlwIjoiSldUIn0";
    const PAYLOAD_ALPHA: &str = "eyJzdWIiOiJ1c2VyXzEyMyIsImVtYWlsIjoiYWxwaGFAZXhhbXBsZS5jb20iLCJodHRwczovL2FwaS5vcGVuYWkuY29tL2F1dGgiOnsiY2hhdGdwdF91c2VyX2lkIjoidXNlcl8xMjMiLCJlbWFpbCI6ImFscGhhQGV4YW1wbGUuY29tIn19";
    const PAYLOAD_BETA: &str = "eyJzdWIiOiJ1c2VyXzQ1NiIsImVtYWlsIjoiYmV0YUBleGFtcGxlLmNvbSIsImh0dHBzOi8vYXBpLm9wZW5haS5jb20vYXV0aCI6eyJjaGF0Z3B0X3VzZXJfaWQiOiJ1c2VyXzQ1NiIsImVtYWlsIjoiYmV0YUBleGFtcGxlLmNvbSJ9fQ";

    fn token(payload: &str) -> String {
        format!("{HEADER}.{payload}.sig")
    }

    fn auth_json(payload: &str) -> String {
        format!(
            r#"{{"tokens":{{"id_token":"{}","access_token":"{}"}}}}"#,
            token(payload),
            token(payload)
        )
    }

    struct EnvGuard {
        key: &'static str,
        old: Option<std::ffi::OsString>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
            let old = std::env::var_os(key);
            std::env::set_var(key, value);
            Self { key, old }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            if let Some(value) = self.old.take() {
                std::env::set_var(self.key, value);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

    #[test]
    fn resolve_by_email_supports_full_and_local_part_lookup() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        std::fs::write(dir.path().join("alpha.json"), auth_json(PAYLOAD_ALPHA)).expect("alpha");
        std::fs::write(dir.path().join("beta.json"), auth_json(PAYLOAD_BETA)).expect("beta");

        match resolve_by_email(dir.path(), "alpha@example.com") {
            ResolveResult::Exact(name) => assert_eq!(name, "alpha.json"),
            other => panic!("expected exact alpha match, got {other:?}"),
        }
        match resolve_by_email(dir.path(), "beta") {
            ResolveResult::Exact(name) => assert_eq!(name, "beta.json"),
            other => panic!("expected exact beta match, got {other:?}"),
        }
    }

    #[test]
    fn resolve_by_email_reports_ambiguous_and_not_found() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        std::fs::write(dir.path().join("alpha-1.json"), auth_json(PAYLOAD_ALPHA)).expect("alpha-1");
        std::fs::write(dir.path().join("alpha-2.json"), auth_json(PAYLOAD_ALPHA)).expect("alpha-2");

        match resolve_by_email(dir.path(), "alpha@example.com") {
            ResolveResult::Ambiguous { candidates } => {
                assert_eq!(candidates.len(), 2);
                assert!(candidates.contains(&"alpha-1.json".to_string()));
                assert!(candidates.contains(&"alpha-2.json".to_string()));
            }
            other => panic!("expected ambiguous match, got {other:?}"),
        }

        match resolve_by_email(dir.path(), "missing@example.com") {
            ResolveResult::NotFound => {}
            other => panic!("expected not found, got {other:?}"),
        }
    }

    #[test]
    fn secret_timestamp_path_uses_cache_dir_and_default_file_name() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let cache = dir.path().join("cache");
        std::fs::create_dir_all(&cache).expect("cache");
        let _guard = EnvGuard::set("CODEX_SECRET_CACHE_DIR", &cache);

        let with_name =
            secret_timestamp_path(Path::new("/tmp/demo-auth.json")).expect("timestamp path");
        assert_eq!(with_name, cache.join("demo-auth.json.timestamp"));

        let without_name = secret_timestamp_path(Path::new("")).expect("timestamp path");
        assert_eq!(without_name, cache.join("auth.json.timestamp"));
    }

    #[test]
    fn file_name_returns_empty_when_path_has_no_file_name() {
        assert_eq!(file_name(Path::new("a/b/c.json")), "c.json");
        assert_eq!(file_name(Path::new("")), "");
    }
}
