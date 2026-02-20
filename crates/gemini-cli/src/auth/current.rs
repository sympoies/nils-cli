use std::io::ErrorKind;
use std::path::Path;

use crate::auth;
use crate::auth::output;

pub fn run() -> i32 {
    run_with_json(false)
}

pub fn run_with_json(output_json: bool) -> i32 {
    let auth_file = match gemini_core::paths::resolve_auth_file() {
        Some(path) => path,
        None => {
            if output_json {
                let _ = output::emit_error(
                    "auth current",
                    "auth-file-not-configured",
                    "GEMINI_AUTH_FILE is not configured",
                    None,
                );
            }
            return 1;
        }
    };

    if !auth_file.is_file() {
        if output_json {
            let _ = output::emit_error(
                "auth current",
                "auth-file-not-found",
                format!("{} not found", auth_file.display()),
                Some(output::obj(vec![(
                    "auth_file",
                    output::s(auth_file.display().to_string()),
                )])),
            );
        } else {
            eprintln!("gemini: {} not found", auth_file.display());
        }
        return 1;
    }

    let auth_key = auth::identity_key_from_auth_file(&auth_file).ok().flatten();
    let auth_contents = match std::fs::read(&auth_file) {
        Ok(bytes) => bytes,
        Err(_) => {
            eprintln!("gemini: failed to read {}", auth_file.display());
            return 1;
        }
    };

    let secret_dir = match gemini_core::paths::resolve_secret_dir() {
        Some(path) => path,
        None => {
            emit_secret_dir_error(
                output_json,
                "secret-dir-not-configured",
                "GEMINI_SECRET_DIR is not configured".to_string(),
                None,
            );
            return 1;
        }
    };

    let entries = match std::fs::read_dir(&secret_dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == ErrorKind::NotFound => {
            emit_secret_dir_error(
                output_json,
                "secret-dir-not-found",
                format!("{} not found", secret_dir.display()),
                Some(output::obj(vec![(
                    "secret_dir",
                    output::s(secret_dir.display().to_string()),
                )])),
            );
            return 1;
        }
        Err(err) => {
            emit_secret_dir_error(
                output_json,
                "secret-dir-read-failed",
                format!("failed to read {}: {err}", secret_dir.display()),
                Some(output::obj(vec![
                    ("secret_dir", output::s(secret_dir.display().to_string())),
                    ("error", output::s(err.to_string())),
                ])),
            );
            return 1;
        }
    };

    let mut matched: Option<(String, MatchMode)> = None;

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }

        if let Some(key) = auth_key.as_deref()
            && let Ok(Some(candidate_key)) = auth::identity_key_from_auth_file(&path)
            && candidate_key == key
        {
            let candidate_contents = match std::fs::read(&path) {
                Ok(bytes) => bytes,
                Err(_) => {
                    if output_json {
                        let _ = output::emit_error(
                            "auth current",
                            "secret-read-failed",
                            format!("failed to read {}", path.display()),
                            Some(output::obj(vec![(
                                "path",
                                output::s(path.display().to_string()),
                            )])),
                        );
                    } else {
                        eprintln!("gemini: failed to read {}", path.display());
                    }
                    return 1;
                }
            };

            let mode = if candidate_contents == auth_contents {
                MatchMode::Exact
            } else {
                MatchMode::Identity
            };
            matched = Some((file_name(&path), mode));
            break;
        }

        let candidate_contents = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(_) => {
                if output_json {
                    let _ = output::emit_error(
                        "auth current",
                        "secret-read-failed",
                        format!("failed to read {}", path.display()),
                        Some(output::obj(vec![(
                            "path",
                            output::s(path.display().to_string()),
                        )])),
                    );
                } else {
                    eprintln!("gemini: failed to read {}", path.display());
                }
                return 1;
            }
        };

        if candidate_contents == auth_contents {
            matched = Some((file_name(&path), MatchMode::Exact));
            break;
        }
    }

    if let Some((secret_name, mode)) = matched {
        if output_json {
            let match_mode = match mode {
                MatchMode::Exact => "exact",
                MatchMode::Identity => "identity",
            };
            let _ = output::emit_result(
                "auth current",
                output::obj(vec![
                    ("auth_file", output::s(auth_file.display().to_string())),
                    ("matched", output::b(true)),
                    ("matched_secret", output::s(secret_name)),
                    ("match_mode", output::s(match_mode)),
                ]),
            );
        } else {
            match mode {
                MatchMode::Exact => {
                    println!("gemini: {} matches {}", auth_file.display(), secret_name);
                }
                MatchMode::Identity => {
                    println!(
                        "gemini: {} matches {} (identity; secret differs)",
                        auth_file.display(),
                        secret_name
                    );
                }
            }
        }
        return 0;
    }

    if output_json {
        let _ = output::emit_error(
            "auth current",
            "secret-not-matched",
            format!("{} does not match any known secret", auth_file.display()),
            Some(output::obj(vec![
                ("auth_file", output::s(auth_file.display().to_string())),
                ("matched", output::b(false)),
            ])),
        );
    } else {
        println!(
            "gemini: {} does not match any known secret",
            auth_file.display()
        );
    }
    2
}

#[derive(Copy, Clone)]
enum MatchMode {
    Exact,
    Identity,
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_string()
}

fn emit_secret_dir_error(
    output_json: bool,
    code: &str,
    message: String,
    details: Option<output::JsonValue>,
) {
    if output_json {
        let _ = output::emit_error("auth current", code, message, details);
    } else {
        eprintln!("gemini: {message}");
    }
}
