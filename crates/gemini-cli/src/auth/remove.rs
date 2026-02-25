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
            "gemini-remove: usage: gemini-remove [--yes] <secret.json>",
        );
    }

    if is_invalid_target(target) {
        if output_json {
            let _ = output::emit_error(
                "auth remove",
                "invalid-secret-file-name",
                format!("gemini-remove: invalid secret file name: {target}"),
                Some(output::obj(vec![("target", output::s(target))])),
            );
        } else {
            eprintln!("gemini-remove: invalid secret file name: {target}");
        }
        return 64;
    }

    let secret_dir = match resolve_secret_dir() {
        Some(path) => path,
        None => {
            if output_json {
                let _ = output::emit_error(
                    "auth remove",
                    "secret-dir-not-configured",
                    "gemini-remove: secret directory is not configured",
                    None,
                );
            } else {
                eprintln!("gemini-remove: secret directory is not configured");
            }
            return 1;
        }
    };

    if !secret_dir.is_dir() {
        if output_json {
            let _ = output::emit_error(
                "auth remove",
                "secret-dir-not-found",
                format!(
                    "gemini-remove: secret directory not found: {}",
                    secret_dir.display()
                ),
                Some(output::obj(vec![(
                    "secret_dir",
                    output::s(secret_dir.display().to_string()),
                )])),
            );
        } else {
            eprintln!(
                "gemini-remove: secret directory not found: {}",
                secret_dir.display()
            );
        }
        return 1;
    }

    let target_file = secret_dir.join(target);
    if !target_file.is_file() {
        if output_json {
            let _ = output::emit_error(
                "auth remove",
                "target-not-found",
                format!(
                    "gemini-remove: secret file not found: {}",
                    target_file.display()
                ),
                Some(output::obj(vec![(
                    "target_file",
                    output::s(target_file.display().to_string()),
                )])),
            );
        } else {
            eprintln!(
                "gemini-remove: secret file not found: {}",
                target_file.display()
            );
        }
        return 1;
    }

    if !yes {
        if output_json {
            let _ = output::emit_error(
                "auth remove",
                "remove-confirmation-required",
                format!(
                    "gemini-remove: {} exists; rerun with --yes to remove",
                    target_file.display()
                ),
                Some(output::obj(vec![
                    ("target_file", output::s(target_file.display().to_string())),
                    ("removed", output::b(false)),
                ])),
            );
            return 1;
        }

        if !interactive_io_available() {
            eprintln!(
                "gemini-remove: {} exists; rerun with --yes to remove",
                target_file.display()
            );
            return 1;
        }

        match confirm_remove(&target_file) {
            Ok(true) => {}
            Ok(false) => {
                eprintln!(
                    "gemini-remove: removal declined for {}",
                    target_file.display()
                );
                return 1;
            }
            Err(_) => return 1,
        }
    }

    if let Err(err) = std::fs::remove_file(&target_file) {
        if output_json {
            let _ = output::emit_error(
                "auth remove",
                "remove-failed",
                format!("gemini-remove: failed to remove {}", target_file.display()),
                Some(output::obj(vec![
                    ("target_file", output::s(target_file.display().to_string())),
                    ("error", output::s(err.to_string())),
                ])),
            );
        } else {
            eprintln!("gemini-remove: failed to remove {}", target_file.display());
        }
        return 1;
    }

    remove_target_timestamp(&target_file);

    if output_json {
        let _ = output::emit_result(
            "auth remove",
            output::obj(vec![
                ("target_file", output::s(target_file.display().to_string())),
                ("removed", output::b(true)),
            ]),
        );
    } else {
        println!("gemini: removed {}", target_file.display());
    }
    0
}

fn usage_error(output_json: bool, message: &str) -> i32 {
    if output_json {
        let _ = output::emit_error("auth remove", "invalid-usage", message, None);
    } else {
        eprintln!("{message}");
    }
    64
}

fn resolve_secret_dir() -> Option<PathBuf> {
    crate::paths::resolve_secret_dir()
}

fn is_invalid_target(target: &str) -> bool {
    target.contains('/') || target.contains('\\') || target.contains("..")
}

fn interactive_io_available() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal()
}

fn confirm_remove(target: &Path) -> io::Result<bool> {
    eprint!("gemini-remove: remove {}? [y/N]: ", target.display());
    io::stderr().flush()?;

    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    let normalized = line.trim().to_ascii_lowercase();
    Ok(matches!(normalized.as_str(), "y" | "yes"))
}

fn remove_target_timestamp(target_file: &Path) {
    let Some(cache_dir) = crate::paths::resolve_secret_cache_dir() else {
        return;
    };
    let file_name = target_file
        .file_name()
        .and_then(|v| v.to_str())
        .unwrap_or("auth.json");
    let timestamp_file = cache_dir.join(format!("{file_name}.timestamp"));
    let _ = auth::write_timestamp(&timestamp_file, None);
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
        let _lock = crate::auth::test_env_lock();
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
