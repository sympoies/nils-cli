use anyhow::Result;
use serde_json::json;
use std::path::{Path, PathBuf};

use crate::auth;
use crate::auth::output::{self, AuthUseResult};
use crate::paths;
use nils_common::fs;

pub fn run(target: &str) -> Result<i32> {
    run_with_json(target, false)
}

pub fn run_with_json(target: &str, output_json: bool) -> Result<i32> {
    if target.is_empty() {
        if output_json {
            output::emit_error(
                "auth use",
                "invalid-usage",
                "codex-use: usage: codex-use <name|name.json|email>",
                None,
            )?;
        } else {
            eprintln!("codex-use: usage: codex-use <name|name.json|email>");
        }
        return Ok(64);
    }

    if auth::is_invalid_secret_target(target) {
        if output_json {
            output::emit_error(
                "auth use",
                "invalid-secret-name",
                format!("codex-use: invalid secret name: {target}"),
                Some(json!({
                    "target": target,
                })),
            )?;
        } else {
            eprintln!("codex-use: invalid secret name: {target}");
        }
        return Ok(64);
    }

    let secret_dir = match paths::resolve_secret_dir() {
        Some(dir) => dir,
        None => {
            if output_json {
                output::emit_error(
                    "auth use",
                    "secret-not-found",
                    format!("codex-use: secret not found: {target}"),
                    Some(json!({
                        "target": target,
                    })),
                )?;
            } else {
                eprintln!("codex-use: secret not found: {target}");
            }
            return Ok(1);
        }
    };

    let is_email = target.contains('@');
    let secret_name = if is_email {
        target.to_string()
    } else {
        auth::normalize_secret_file_name(target)
    };

    if secret_dir.join(&secret_name).is_file() {
        let (code, auth_file) = apply_secret(&secret_dir, &secret_name, output_json)?;
        if output_json && code == 0 {
            output::emit_result(
                "auth use",
                AuthUseResult {
                    target: target.to_string(),
                    matched_secret: Some(secret_name),
                    applied: true,
                    auth_file: auth_file.unwrap_or_default(),
                },
            )?;
        }
        return Ok(code);
    }

    match resolve_by_email(&secret_dir, target) {
        ResolveResult::Exact(name) => {
            let (code, auth_file) = apply_secret(&secret_dir, &name, output_json)?;
            if output_json && code == 0 {
                output::emit_result(
                    "auth use",
                    AuthUseResult {
                        target: target.to_string(),
                        matched_secret: Some(name),
                        applied: true,
                        auth_file: auth_file.unwrap_or_default(),
                    },
                )?;
            }
            Ok(code)
        }
        ResolveResult::Ambiguous { candidates } => {
            if output_json {
                output::emit_error(
                    "auth use",
                    "ambiguous-secret",
                    format!("codex-use: identifier matches multiple secrets: {target}"),
                    Some(json!({
                        "target": target,
                        "candidates": candidates,
                    })),
                )?;
            } else {
                eprintln!("codex-use: identifier matches multiple secrets: {target}");
                eprintln!("codex-use: candidates: {}", candidates.join(", "));
            }
            Ok(2)
        }
        ResolveResult::NotFound => {
            if output_json {
                output::emit_error(
                    "auth use",
                    "secret-not-found",
                    format!("codex-use: secret not found: {target}"),
                    Some(json!({
                        "target": target,
                    })),
                )?;
            } else {
                eprintln!("codex-use: secret not found: {target}");
            }
            Ok(1)
        }
    }
}

fn apply_secret(
    secret_dir: &Path,
    secret_name: &str,
    output_json: bool,
) -> Result<(i32, Option<String>)> {
    let source_file = secret_dir.join(secret_name);
    if !source_file.is_file() {
        if !output_json {
            eprintln!("codex: secret file {secret_name} not found");
        }
        return Ok((1, None));
    }

    let auth_file = match paths::resolve_auth_file() {
        Some(path) => path,
        None => return Ok((1, None)),
    };

    if auth_file.is_file() {
        let sync_result = crate::auth::sync::run_with_json(false)?;
        if sync_result != 0 {
            if !output_json {
                eprintln!("codex: failed to sync current auth before switching secrets");
            }
            return Ok((1, None));
        }
    }

    let contents = std::fs::read(&source_file)?;
    fs::write_atomic(&auth_file, &contents, fs::SECRET_FILE_MODE)?;

    let iso = auth::last_refresh_from_auth_file(&auth_file).unwrap_or(None);
    let timestamp_path = secret_timestamp_path(&auth_file)?;
    fs::write_timestamp(&timestamp_path, iso.as_deref())?;

    if !output_json {
        println!("codex: applied {secret_name} to {}", auth_file.display());
    }
    Ok((0, Some(auth_file.display().to_string())))
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
            } else if let Some(local_part) = email_lower.split('@').next()
                && local_part == query
            {
                matches.push(file_name(&path));
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
    paths::resolve_secret_timestamp_path(target_file)
        .ok_or_else(|| anyhow::anyhow!("CODEX_SECRET_CACHE_DIR not resolved"))
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
    use super::{ResolveResult, file_name, resolve_by_email, secret_timestamp_path};
    use nils_test_support::{EnvGuard, GlobalStateLock};
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
        let lock = GlobalStateLock::new();
        let dir = tempfile::TempDir::new().expect("tempdir");
        let cache = dir.path().join("cache");
        std::fs::create_dir_all(&cache).expect("cache");
        let cache_value = cache.to_string_lossy().to_string();
        let _guard = EnvGuard::set(&lock, "CODEX_SECRET_CACHE_DIR", &cache_value);

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
