use std::io::Write;

pub fn show() -> i32 {
    let stdout = std::io::stdout();
    let mut stdout = stdout.lock();
    show_with_io(&mut stdout)
}

pub fn show_with_io(stdout: &mut impl Write) -> i32 {
    let snapshot = crate::runtime::config_snapshot();

    let _ = writeln!(stdout, "GEMINI_CLI_MODEL={}", snapshot.model);
    let _ = writeln!(stdout, "GEMINI_CLI_REASONING={}", snapshot.reasoning);
    let _ = writeln!(
        stdout,
        "GEMINI_ALLOW_DANGEROUS_ENABLED={}",
        snapshot.allow_dangerous_enabled_raw
    );

    if let Some(path) = snapshot.secret_dir {
        let _ = writeln!(stdout, "GEMINI_SECRET_DIR={}", path.to_string_lossy());
    } else {
        let _ = writeln!(stdout, "GEMINI_SECRET_DIR=");
    }

    if let Some(path) = snapshot.auth_file {
        let _ = writeln!(stdout, "GEMINI_AUTH_FILE={}", path.to_string_lossy());
    } else {
        let _ = writeln!(stdout, "GEMINI_AUTH_FILE=");
    }

    if let Some(path) = snapshot.secret_cache_dir {
        let _ = writeln!(stdout, "GEMINI_SECRET_CACHE_DIR={}", path.to_string_lossy());
    } else {
        let _ = writeln!(stdout, "GEMINI_SECRET_CACHE_DIR=");
    }

    let _ = writeln!(
        stdout,
        "GEMINI_STARSHIP_ENABLED={}",
        snapshot.starship_enabled
    );
    let _ = writeln!(
        stdout,
        "GEMINI_AUTO_REFRESH_ENABLED={}",
        snapshot.auto_refresh_enabled
    );
    let _ = writeln!(
        stdout,
        "GEMINI_AUTO_REFRESH_MIN_DAYS={}",
        snapshot.auto_refresh_min_days
    );

    0
}

pub fn set(key: &str, value: &str) -> i32 {
    let stdout = std::io::stdout();
    let mut stdout = stdout.lock();
    let stderr = std::io::stderr();
    let mut stderr = stderr.lock();
    set_with_io(key, value, &mut stdout, &mut stderr)
}

pub fn set_with_io(
    key: &str,
    value: &str,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> i32 {
    match key {
        "model" | "GEMINI_CLI_MODEL" => {
            let _ = writeln!(
                stdout,
                "export GEMINI_CLI_MODEL={}",
                quote_posix_single(value)
            );
            0
        }
        "reasoning" | "reason" | "GEMINI_CLI_REASONING" => {
            let _ = writeln!(
                stdout,
                "export GEMINI_CLI_REASONING={}",
                quote_posix_single(value)
            );
            0
        }
        "dangerous" | "allow-dangerous" | "GEMINI_ALLOW_DANGEROUS_ENABLED" => {
            let lowered = value.trim().to_ascii_lowercase();
            if lowered != "true" && lowered != "false" {
                let _ = writeln!(
                    stderr,
                    "gemini-cli config: dangerous must be true|false (got: {})",
                    value
                );
                return 64;
            }
            let _ = writeln!(stdout, "export GEMINI_ALLOW_DANGEROUS_ENABLED={}", lowered);
            0
        }
        _ => {
            let _ = writeln!(stderr, "gemini-cli config: unknown key: {key}");
            let _ = writeln!(stderr, "gemini-cli config: keys: model|reasoning|dangerous");
            64
        }
    }
}

fn quote_posix_single(raw: &str) -> String {
    let escaped = raw.replace('\'', "'\"'\"'");
    format!("'{escaped}'")
}

#[cfg(test)]
mod tests {
    use super::quote_posix_single;

    #[test]
    fn quote_posix_single_handles_single_quotes_and_empty() {
        assert_eq!(quote_posix_single(""), "''");
        assert_eq!(quote_posix_single("abc"), "'abc'");
        assert_eq!(quote_posix_single("a'b"), "'a'\"'\"'b'");
    }
}
