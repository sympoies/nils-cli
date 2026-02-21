use std::path::{Path, PathBuf};

use crate::auth;
use crate::auth::output;

pub fn run(target: &str) -> i32 {
    run_with_json(target, false)
}

pub fn run_with_json(target: &str, output_json: bool) -> i32 {
    if target.is_empty() {
        if output_json {
            let _ = output::emit_error(
                "auth use",
                "invalid-usage",
                "gemini-use: usage: gemini-use <name|name.json|email>",
                None,
            );
        } else {
            eprintln!("gemini-use: usage: gemini-use <name|name.json|email>");
        }
        return 64;
    }

    if target.contains('/') || target.contains("..") {
        if output_json {
            let _ = output::emit_error(
                "auth use",
                "invalid-secret-name",
                format!("gemini-use: invalid secret name: {target}"),
                Some(output::obj(vec![("target", output::s(target))])),
            );
        } else {
            eprintln!("gemini-use: invalid secret name: {target}");
        }
        return 64;
    }

    let secret_dir = match crate::paths::resolve_secret_dir() {
        Some(dir) => dir,
        None => {
            if output_json {
                let _ = output::emit_error(
                    "auth use",
                    "secret-not-found",
                    format!("gemini-use: secret not found: {target}"),
                    Some(output::obj(vec![("target", output::s(target))])),
                );
            } else {
                eprintln!("gemini-use: secret not found: {target}");
            }
            return 1;
        }
    };

    let is_email = target.contains('@');
    let mut secret_name = target.to_string();
    if !secret_name.ends_with(".json") && !is_email {
        secret_name.push_str(".json");
    }

    if secret_dir.join(&secret_name).is_file() {
        let (code, auth_file) = apply_secret(&secret_dir, &secret_name, output_json);
        if output_json && code == 0 {
            let _ = output::emit_result(
                "auth use",
                output::obj(vec![
                    ("target", output::s(target)),
                    ("matched_secret", output::s(secret_name)),
                    ("applied", output::b(true)),
                    ("auth_file", output::s(auth_file.unwrap_or_default())),
                ]),
            );
        }
        return code;
    }

    match resolve_by_email(&secret_dir, target) {
        ResolveResult::Exact(name) => {
            let (code, auth_file) = apply_secret(&secret_dir, &name, output_json);
            if output_json && code == 0 {
                let _ = output::emit_result(
                    "auth use",
                    output::obj(vec![
                        ("target", output::s(target)),
                        ("matched_secret", output::s(name)),
                        ("applied", output::b(true)),
                        ("auth_file", output::s(auth_file.unwrap_or_default())),
                    ]),
                );
            }
            code
        }
        ResolveResult::Ambiguous { candidates } => {
            if output_json {
                let _ = output::emit_error(
                    "auth use",
                    "ambiguous-secret",
                    format!("gemini-use: identifier matches multiple secrets: {target}"),
                    Some(output::obj(vec![
                        ("target", output::s(target)),
                        (
                            "candidates",
                            output::arr(candidates.into_iter().map(output::s).collect()),
                        ),
                    ])),
                );
            } else {
                eprintln!("gemini-use: identifier matches multiple secrets: {target}");
                eprintln!("gemini-use: candidates: {}", candidates.join(", "));
            }
            2
        }
        ResolveResult::NotFound => {
            if output_json {
                let _ = output::emit_error(
                    "auth use",
                    "secret-not-found",
                    format!("gemini-use: secret not found: {target}"),
                    Some(output::obj(vec![("target", output::s(target))])),
                );
            } else {
                eprintln!("gemini-use: secret not found: {target}");
            }
            1
        }
    }
}

fn apply_secret(secret_dir: &Path, secret_name: &str, output_json: bool) -> (i32, Option<String>) {
    let source_file = secret_dir.join(secret_name);
    if !source_file.is_file() {
        if !output_json {
            eprintln!("gemini: secret file {secret_name} not found");
        }
        return (1, None);
    }

    let auth_file = match crate::paths::resolve_auth_file() {
        Some(path) => path,
        None => return (1, None),
    };

    if auth_file.is_file() {
        let sync_result = crate::auth::sync::run_with_json(false);
        if sync_result != 0 {
            if !output_json {
                eprintln!("gemini: failed to sync current auth before switching secrets");
            }
            return (1, None);
        }
    }

    let contents = match std::fs::read(&source_file) {
        Ok(contents) => contents,
        Err(_) => return (1, None),
    };

    if auth::write_atomic(&auth_file, &contents, auth::SECRET_FILE_MODE).is_err() {
        return (1, None);
    }

    let iso = auth::last_refresh_from_auth_file(&auth_file).ok().flatten();
    if let Some(timestamp_path) = secret_timestamp_path(&auth_file) {
        let _ = auth::write_timestamp(&timestamp_path, iso.as_deref());
    }

    if !output_json {
        println!("gemini: applied {secret_name} to {}", auth_file.display());
    }
    (0, Some(auth_file.display().to_string()))
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

fn secret_timestamp_path(target_file: &Path) -> Option<PathBuf> {
    let cache_dir = crate::paths::resolve_secret_cache_dir()?;
    let name = target_file
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("auth.json");
    Some(cache_dir.join(format!("{name}.timestamp")))
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
    use super::{ResolveResult, resolve_by_email};

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
        let dir = std::env::temp_dir().join(format!(
            "gemini-use-test-{}-{}",
            std::process::id(),
            super::super::now_epoch_seconds()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");

        std::fs::write(dir.join("alpha.json"), auth_json(PAYLOAD_ALPHA)).expect("alpha");
        std::fs::write(dir.join("beta.json"), auth_json(PAYLOAD_BETA)).expect("beta");

        match resolve_by_email(&dir, "alpha@example.com") {
            ResolveResult::Exact(name) => assert_eq!(name, "alpha.json"),
            other => panic!("expected exact alpha match, got {other:?}"),
        }
        match resolve_by_email(&dir, "beta") {
            ResolveResult::Exact(name) => assert_eq!(name, "beta.json"),
            other => panic!("expected exact beta match, got {other:?}"),
        }

        let _ = std::fs::remove_dir_all(&dir);
    }
}
