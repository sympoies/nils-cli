use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};

use crate::auth;
use crate::auth::output;

pub fn run(target: &str, yes: bool) -> i32 {
    run_with_json(target, yes, false)
}

pub fn run_with_json(target: &str, yes: bool, output_json: bool) -> i32 {
    if target.is_empty() {
        return usage_error(
            output_json,
            "gemini-save: usage: gemini-save [--yes] <secret.json>",
        );
    }

    if is_invalid_target(target) {
        if output_json {
            let _ = output::emit_error(
                "auth save",
                "invalid-secret-file-name",
                format!("gemini-save: invalid secret file name: {target}"),
                Some(output::obj(vec![("target", output::s(target))])),
            );
        } else {
            eprintln!("gemini-save: invalid secret file name: {target}");
        }
        return 64;
    }

    let secret_dir = match resolve_secret_dir() {
        Some(path) => path,
        None => {
            if output_json {
                let _ = output::emit_error(
                    "auth save",
                    "secret-dir-not-configured",
                    "gemini-save: secret directory is not configured",
                    None,
                );
            } else {
                eprintln!("gemini-save: secret directory is not configured");
            }
            return 1;
        }
    };

    if !secret_dir.is_dir() {
        if output_json {
            let _ = output::emit_error(
                "auth save",
                "secret-dir-not-found",
                format!(
                    "gemini-save: secret directory not found: {}",
                    secret_dir.display()
                ),
                Some(output::obj(vec![(
                    "secret_dir",
                    output::s(secret_dir.display().to_string()),
                )])),
            );
        } else {
            eprintln!(
                "gemini-save: secret directory not found: {}",
                secret_dir.display()
            );
        }
        return 1;
    }

    let auth_file = match gemini_core::paths::resolve_auth_file() {
        Some(path) => path,
        None => {
            if output_json {
                let _ = output::emit_error(
                    "auth save",
                    "auth-file-not-configured",
                    "gemini-save: GEMINI_AUTH_FILE is not configured",
                    None,
                );
            } else {
                eprintln!("gemini-save: GEMINI_AUTH_FILE is not configured");
            }
            return 1;
        }
    };

    if !auth_file.is_file() {
        if output_json {
            let _ = output::emit_error(
                "auth save",
                "auth-file-not-found",
                format!("gemini-save: auth file not found: {}", auth_file.display()),
                Some(output::obj(vec![(
                    "auth_file",
                    output::s(auth_file.display().to_string()),
                )])),
            );
        } else {
            eprintln!("gemini-save: auth file not found: {}", auth_file.display());
        }
        return 1;
    }

    let target_file = secret_dir.join(target);
    let mut overwritten = false;
    if target_file.exists() {
        if yes {
            overwritten = true;
        } else if output_json {
            let _ = output::emit_error(
                "auth save",
                "overwrite-confirmation-required",
                format!(
                    "gemini-save: {} exists; rerun with --yes to overwrite",
                    target_file.display()
                ),
                Some(output::obj(vec![
                    ("target_file", output::s(target_file.display().to_string())),
                    ("overwritten", output::b(false)),
                ])),
            );
            return 1;
        } else if !interactive_io_available() {
            eprintln!(
                "gemini-save: {} exists; rerun with --yes to overwrite",
                target_file.display()
            );
            return 1;
        } else {
            match confirm_overwrite(&target_file) {
                Ok(true) => {
                    overwritten = true;
                }
                Ok(false) => {
                    eprintln!(
                        "gemini-save: overwrite declined for {}",
                        target_file.display()
                    );
                    return 1;
                }
                Err(_) => return 1,
            }
        }
    }

    let content = match std::fs::read(&auth_file) {
        Ok(content) => content,
        Err(_) => {
            if output_json {
                let _ = output::emit_error(
                    "auth save",
                    "auth-file-read-failed",
                    format!(
                        "gemini-save: failed to read auth file: {}",
                        auth_file.display()
                    ),
                    Some(output::obj(vec![(
                        "auth_file",
                        output::s(auth_file.display().to_string()),
                    )])),
                );
            } else {
                eprintln!(
                    "gemini-save: failed to read auth file: {}",
                    auth_file.display()
                );
            }
            return 1;
        }
    };

    if let Err(err) = auth::write_atomic(&target_file, &content, auth::SECRET_FILE_MODE) {
        if output_json {
            let _ = output::emit_error(
                "auth save",
                "save-write-failed",
                format!(
                    "gemini-save: failed to write target file {}",
                    target_file.display()
                ),
                Some(output::obj(vec![
                    ("target_file", output::s(target_file.display().to_string())),
                    ("error", output::s(err.to_string())),
                ])),
            );
        } else {
            eprintln!(
                "gemini-save: failed to write target file {}",
                target_file.display()
            );
        }
        return 1;
    }

    let _ = write_target_timestamp(&target_file, &auth_file);

    if output_json {
        let _ = output::emit_result(
            "auth save",
            output::obj(vec![
                ("auth_file", output::s(auth_file.display().to_string())),
                ("target_file", output::s(target_file.display().to_string())),
                ("saved", output::b(true)),
                ("overwritten", output::b(overwritten)),
            ]),
        );
    } else {
        println!(
            "gemini: saved {} to {}{}",
            auth_file.display(),
            target_file.display(),
            if overwritten { " (overwritten)" } else { "" }
        );
    }

    0
}

fn usage_error(output_json: bool, message: &str) -> i32 {
    if output_json {
        let _ = output::emit_error("auth save", "invalid-usage", message, None);
    } else {
        eprintln!("{message}");
    }
    64
}

fn resolve_secret_dir() -> Option<PathBuf> {
    gemini_core::paths::resolve_secret_dir()
}

fn is_invalid_target(target: &str) -> bool {
    target.contains('/') || target.contains('\\') || target.contains("..")
}

fn interactive_io_available() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal()
}

fn confirm_overwrite(target: &Path) -> io::Result<bool> {
    eprint!(
        "gemini-save: {} exists. overwrite? [y/N]: ",
        target.display()
    );
    io::stderr().flush()?;

    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    let normalized = line.trim().to_ascii_lowercase();
    Ok(matches!(normalized.as_str(), "y" | "yes"))
}

fn write_target_timestamp(target_file: &Path, auth_file: &Path) -> io::Result<()> {
    let cache_dir = match gemini_core::paths::resolve_secret_cache_dir() {
        Some(dir) => dir,
        None => return Ok(()),
    };

    let file_name = target_file
        .file_name()
        .and_then(|v| v.to_str())
        .unwrap_or("auth.json");
    let timestamp_file = cache_dir.join(format!("{file_name}.timestamp"));
    let iso = auth::last_refresh_from_auth_file(auth_file).ok().flatten();
    auth::write_timestamp(&timestamp_file, iso.as_deref())
}

#[cfg(test)]
mod tests {
    use super::{is_invalid_target, resolve_secret_dir};

    #[test]
    fn invalid_target_rejects_paths_and_traversal() {
        assert!(is_invalid_target("../a.json"));
        assert!(is_invalid_target("a/b.json"));
        assert!(is_invalid_target(r"a\b.json"));
        assert!(!is_invalid_target("alpha.json"));
    }

    #[test]
    fn resolve_secret_dir_uses_gemini_secret_dir_env_override() {
        let old_home = std::env::var_os("HOME");
        let old = std::env::var_os("GEMINI_SECRET_DIR");
        // SAFETY: test-scoped env mutation.
        unsafe { std::env::set_var("HOME", "") };
        // SAFETY: test-scoped env mutation.
        unsafe { std::env::set_var("GEMINI_SECRET_DIR", "/tmp/secrets") };
        assert_eq!(
            resolve_secret_dir().expect("secret dir"),
            std::path::PathBuf::from("/tmp/secrets")
        );
        if let Some(value) = old_home {
            // SAFETY: test-scoped env restore.
            unsafe { std::env::set_var("HOME", value) };
        } else {
            // SAFETY: test-scoped env restore.
            unsafe { std::env::remove_var("HOME") };
        }
        if let Some(value) = old {
            // SAFETY: test-scoped env restore.
            unsafe { std::env::set_var("GEMINI_SECRET_DIR", value) };
        } else {
            // SAFETY: test-scoped env restore.
            unsafe { std::env::remove_var("GEMINI_SECRET_DIR") };
        }
    }
}
