use crate::paths;
use nils_common::shell::{SingleQuoteEscapeStyle, quote_posix_single_with_style};

pub fn show() -> i32 {
    println!(
        "CODEX_CLI_MODEL={}",
        env_or_default("CODEX_CLI_MODEL", "gpt-5.1-codex-mini")
    );
    println!(
        "CODEX_CLI_REASONING={}",
        env_or_default("CODEX_CLI_REASONING", "medium")
    );
    println!(
        "CODEX_ALLOW_DANGEROUS_ENABLED={}",
        std::env::var("CODEX_ALLOW_DANGEROUS_ENABLED").unwrap_or_default()
    );

    if let Some(path) = paths::resolve_secret_dir() {
        println!("CODEX_SECRET_DIR={}", path.to_string_lossy());
    } else {
        println!("CODEX_SECRET_DIR=");
    }

    if let Some(path) = paths::resolve_auth_file() {
        println!("CODEX_AUTH_FILE={}", path.to_string_lossy());
    } else {
        println!("CODEX_AUTH_FILE=");
    }

    if let Some(path) = paths::resolve_secret_cache_dir() {
        println!("CODEX_SECRET_CACHE_DIR={}", path.to_string_lossy());
    } else {
        println!("CODEX_SECRET_CACHE_DIR=");
    }

    println!(
        "CODEX_AUTO_REFRESH_ENABLED={}",
        env_or_default("CODEX_AUTO_REFRESH_ENABLED", "false")
    );
    println!(
        "CODEX_AUTO_REFRESH_MIN_DAYS={}",
        env_or_default("CODEX_AUTO_REFRESH_MIN_DAYS", "5")
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

fn env_or_default(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}
