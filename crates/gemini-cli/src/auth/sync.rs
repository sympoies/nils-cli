use std::path::{Path, PathBuf};

use crate::auth;
use crate::auth::output;

pub fn run() -> i32 {
    run_with_json(false)
}

pub fn run_with_json(output_json: bool) -> i32 {
    let auth_file = match crate::paths::resolve_auth_file() {
        Some(path) => path,
        None => {
            if output_json {
                let _ = output::emit_result(
                    "auth sync",
                    output::obj(vec![
                        ("auth_file", output::s("")),
                        ("synced", output::n(0)),
                        ("skipped", output::n(0)),
                        ("failed", output::n(0)),
                        ("updated_files", output::arr(Vec::new())),
                    ]),
                );
            }
            return 0;
        }
    };

    if !auth_file.is_file() {
        if output_json {
            let _ = output::emit_result(
                "auth sync",
                output::obj(vec![
                    ("auth_file", output::s(auth_file.display().to_string())),
                    ("synced", output::n(0)),
                    ("skipped", output::n(1)),
                    ("failed", output::n(0)),
                    ("updated_files", output::arr(Vec::new())),
                ]),
            );
        }
        return 0;
    }

    let auth_key = match auth::identity_key_from_auth_file(&auth_file) {
        Ok(Some(key)) => key,
        _ => {
            if output_json {
                let _ = output::emit_result(
                    "auth sync",
                    output::obj(vec![
                        ("auth_file", output::s(auth_file.display().to_string())),
                        ("synced", output::n(0)),
                        ("skipped", output::n(1)),
                        ("failed", output::n(0)),
                        ("updated_files", output::arr(Vec::new())),
                    ]),
                );
            }
            return 0;
        }
    };

    let auth_last_refresh = auth::last_refresh_from_auth_file(&auth_file).ok().flatten();
    let auth_contents = match std::fs::read(&auth_file) {
        Ok(contents) => contents,
        Err(_) => {
            if output_json {
                let _ = output::emit_error(
                    "auth sync",
                    "auth-read-failed",
                    format!("failed to read {}", auth_file.display()),
                    Some(output::obj(vec![(
                        "path",
                        output::s(auth_file.display().to_string()),
                    )])),
                );
            } else {
                eprintln!("gemini: failed to read {}", auth_file.display());
            }
            return 1;
        }
    };

    let mut synced = 0usize;
    let mut skipped = 0usize;
    let failed = 0usize;
    let mut updated_files: Vec<String> = Vec::new();

    if let Some(secret_dir) = crate::paths::resolve_secret_dir()
        && let Ok(entries) = std::fs::read_dir(&secret_dir)
    {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }

            let candidate_key = match auth::identity_key_from_auth_file(&path) {
                Ok(Some(key)) => key,
                _ => {
                    skipped += 1;
                    continue;
                }
            };

            if candidate_key != auth_key {
                skipped += 1;
                continue;
            }

            let secret_contents = match std::fs::read(&path) {
                Ok(contents) => contents,
                Err(_) => {
                    if output_json {
                        let _ = output::emit_error(
                            "auth sync",
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

            if secret_contents == auth_contents {
                skipped += 1;
                continue;
            }

            if auth::write_atomic(&path, &auth_contents, auth::SECRET_FILE_MODE).is_err() {
                if output_json {
                    let _ = output::emit_error(
                        "auth sync",
                        "sync-write-failed",
                        format!("failed to write {}", path.display()),
                        Some(output::obj(vec![(
                            "path",
                            output::s(path.display().to_string()),
                        )])),
                    );
                } else {
                    eprintln!("gemini: failed to write {}", path.display());
                }
                return 1;
            }

            if let Some(timestamp_path) = secret_timestamp_path(&path) {
                let _ = auth::write_timestamp(&timestamp_path, auth_last_refresh.as_deref());
            }
            synced += 1;
            updated_files.push(path.display().to_string());
        }
    }

    if let Some(auth_timestamp) = secret_timestamp_path(&auth_file) {
        let _ = auth::write_timestamp(&auth_timestamp, auth_last_refresh.as_deref());
    }

    if output_json {
        let _ = output::emit_result(
            "auth sync",
            output::obj(vec![
                ("auth_file", output::s(auth_file.display().to_string())),
                ("synced", output::n(synced as i64)),
                ("skipped", output::n(skipped as i64)),
                ("failed", output::n(failed as i64)),
                (
                    "updated_files",
                    output::arr(updated_files.into_iter().map(output::s).collect()),
                ),
            ]),
        );
    }

    0
}

fn secret_timestamp_path(target_file: &Path) -> Option<PathBuf> {
    let cache_dir = crate::paths::resolve_secret_cache_dir()?;
    let name = target_file
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("auth.json");
    Some(cache_dir.join(format!("{name}.timestamp")))
}
