use nils_common::shell::{SingleQuoteEscapeStyle, quote_posix_single_with_style};

pub fn show() -> i32 {
    let snapshot = codex_core::config::snapshot();

    println!("CODEX_CLI_MODEL={}", snapshot.model);
    println!("CODEX_CLI_REASONING={}", snapshot.reasoning);
    println!(
        "CODEX_ALLOW_DANGEROUS_ENABLED={}",
        snapshot.allow_dangerous_enabled_raw
    );

    if let Some(path) = snapshot.secret_dir {
        println!("CODEX_SECRET_DIR={}", path.to_string_lossy());
    } else {
        println!("CODEX_SECRET_DIR=");
    }

    if let Some(path) = snapshot.auth_file {
        println!("CODEX_AUTH_FILE={}", path.to_string_lossy());
    } else {
        println!("CODEX_AUTH_FILE=");
    }

    if let Some(path) = snapshot.secret_cache_dir {
        println!("CODEX_SECRET_CACHE_DIR={}", path.to_string_lossy());
    } else {
        println!("CODEX_SECRET_CACHE_DIR=");
    }

    println!("CODEX_STARSHIP_ENABLED={}", snapshot.starship_enabled);
    println!(
        "CODEX_AUTO_REFRESH_ENABLED={}",
        snapshot.auto_refresh_enabled
    );
    println!(
        "CODEX_AUTO_REFRESH_MIN_DAYS={}",
        snapshot.auto_refresh_min_days
    );

    0
}

pub fn set(key: &str, value: &str) -> i32 {
    match key {
        "model" | "CODEX_CLI_MODEL" => {
            println!(
                "export CODEX_CLI_MODEL={}",
                quote_posix_single_with_style(value, SingleQuoteEscapeStyle::DoubleQuoteBoundary)
            );
            0
        }
        "reasoning" | "reason" | "CODEX_CLI_REASONING" => {
            println!(
                "export CODEX_CLI_REASONING={}",
                quote_posix_single_with_style(value, SingleQuoteEscapeStyle::DoubleQuoteBoundary)
            );
            0
        }
        "dangerous" | "allow-dangerous" | "CODEX_ALLOW_DANGEROUS_ENABLED" => {
            let lowered = value.trim().to_ascii_lowercase();
            if lowered != "true" && lowered != "false" {
                eprintln!(
                    "codex-cli config: dangerous must be true|false (got: {})",
                    value
                );
                return 64;
            }
            println!("export CODEX_ALLOW_DANGEROUS_ENABLED={}", lowered);
            0
        }
        _ => {
            eprintln!("codex-cli config: unknown key: {key}");
            eprintln!("codex-cli config: keys: model|reasoning|dangerous");
            64
        }
    }
}
