use crate::auth::remote;
use nils_common::shell::{SingleQuoteEscapeStyle, quote_posix_single_with_style};
use std::fs;
use std::io::Write;
use std::path::PathBuf;

fn config_path() -> Result<PathBuf, String> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .ok_or_else(|| "HOME is unavailable".to_string())?;
    Ok(base.join("codex-cli/config.toml"))
}

fn persisted_model() -> Result<Option<String>, String> {
    let path = config_path()?;
    let content = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("failed to read {}: {error}", path.display())),
    };
    let table: toml::Table =
        toml::from_str(&content).map_err(|error| format!("invalid {}: {error}", path.display()))?;
    match table.get("model") {
        Some(toml::Value::String(model)) => Ok(Some(model.clone())),
        None => Ok(None),
        Some(_) => Err(format!("model must be a string in {}", path.display())),
    }
}

pub fn effective_model() -> String {
    if let Ok(model) = std::env::var("CODEX_CLI_MODEL") {
        return model;
    }
    match persisted_model() {
        Ok(Some(model)) => model,
        Ok(None) => default_model(),
        Err(message) => {
            eprintln!("codex-cli config: {message}; using built-in model");
            default_model()
        }
    }
}

fn default_model() -> String {
    crate::provider_profile::CODEX_PROVIDER_PROFILE
        .defaults
        .model
        .to_string()
}

fn save_model(value: &str) -> Result<PathBuf, String> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'/'))
    {
        return Err("model must be a nonempty model identifier (letters, digits, ._-/)".into());
    }
    let path = config_path()?;
    let parent = path
        .parent()
        .ok_or_else(|| "invalid config path".to_string())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
    let mut table = match fs::read_to_string(&path) {
        Ok(content) => toml::from_str::<toml::Table>(&content)
            .map_err(|error| format!("invalid {}: {error}", path.display()))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => toml::Table::new(),
        Err(error) => return Err(format!("failed to read {}: {error}", path.display())),
    };
    table.insert("model".to_string(), toml::Value::String(value.to_string()));
    let rendered = toml::to_string(&table).map_err(|error| error.to_string())?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .map_err(|error| format!("failed to create config temp file: {error}"))?;
    temporary
        .write_all(rendered.as_bytes())
        .map_err(|error| format!("failed to write config temp file: {error}"))?;
    temporary
        .persist(&path)
        .map_err(|error| format!("failed to save {}: {error}", path.display()))?;
    Ok(path)
}

pub fn show() -> i32 {
    let snapshot = crate::runtime::config_snapshot();

    println!("CODEX_CLI_MODEL={}", snapshot.model);
    println!("CODEX_CLI_REASONING={}", snapshot.reasoning);
    println!(
        "CODEX_CLI_AGENT_RUNTIME={}",
        std::env::var("CODEX_CLI_AGENT_RUNTIME").unwrap_or_default()
    );
    println!(
        "CODEX_CLI_EPHEMERAL_ENABLED={}",
        std::env::var("CODEX_CLI_EPHEMERAL_ENABLED").unwrap_or_default()
    );
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

    println!(
        "CODEX_PROMPT_SEGMENT_ENABLED={}",
        snapshot.prompt_segment_enabled
    );
    println!(
        "CODEX_AUTO_REFRESH_ENABLED={}",
        snapshot.auto_refresh_enabled
    );
    println!(
        "CODEX_AUTO_REFRESH_MIN_DAYS={}",
        snapshot.auto_refresh_min_days
    );
    println!(
        "{}={}",
        remote::ENV_AUTH_REMOTE_SSH,
        std::env::var(remote::ENV_AUTH_REMOTE_SSH).unwrap_or_default()
    );
    println!(
        "{}={}",
        remote::ENV_AUTH_REMOTE_NAME,
        std::env::var(remote::ENV_AUTH_REMOTE_NAME).unwrap_or_default()
    );
    println!(
        "{}={}",
        remote::ENV_AUTH_REMOTE_REFRESH,
        std::env::var(remote::ENV_AUTH_REMOTE_REFRESH).unwrap_or_default()
    );

    0
}

pub fn set(key: &str, value: &str, persist: bool) -> i32 {
    if persist {
        if key != "model" && key != "CODEX_CLI_MODEL" {
            eprintln!("codex-cli config: --persist currently supports model only");
            return 64;
        }
        return match save_model(value) {
            Ok(path) => {
                println!("saved CODEX_CLI_MODEL to {}", path.display());
                if let Ok(active) = std::env::var("CODEX_CLI_MODEL")
                    && active != value
                {
                    eprintln!(
                        "codex-cli config: CODEX_CLI_MODEL is currently set in the environment; unset it to use the saved model"
                    );
                }
                0
            }
            Err(message) => {
                eprintln!("codex-cli config: {message}");
                1
            }
        };
    }
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
        "agent-runtime" | "agent_runtime" | "CODEX_CLI_AGENT_RUNTIME" => {
            let lowered = value.trim().to_ascii_lowercase();
            if lowered != "isolated" && lowered != "inherited" {
                eprintln!(
                    "codex-cli config: agent-runtime must be isolated|inherited (got: {})",
                    value
                );
                return 64;
            }
            println!("export CODEX_CLI_AGENT_RUNTIME={}", lowered);
            0
        }
        "ephemeral" | "CODEX_CLI_EPHEMERAL_ENABLED" => {
            let lowered = value.trim().to_ascii_lowercase();
            if lowered != "true" && lowered != "false" {
                eprintln!(
                    "codex-cli config: ephemeral must be true|false (got: {})",
                    value
                );
                return 64;
            }
            println!("export CODEX_CLI_EPHEMERAL_ENABLED={}", lowered);
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
        "remote-ssh" | "remote_ssh" | "CODEX_AUTH_REMOTE_SSH" => {
            println!(
                "export {}={}",
                remote::ENV_AUTH_REMOTE_SSH,
                quote_posix_single_with_style(value, SingleQuoteEscapeStyle::DoubleQuoteBoundary)
            );
            0
        }
        "remote-name" | "remote_name" | "CODEX_AUTH_REMOTE_NAME" => {
            println!(
                "export {}={}",
                remote::ENV_AUTH_REMOTE_NAME,
                quote_posix_single_with_style(value, SingleQuoteEscapeStyle::DoubleQuoteBoundary)
            );
            0
        }
        "remote-refresh" | "remote_refresh" | "CODEX_AUTH_REMOTE_REFRESH" => {
            let lowered = value.trim().to_ascii_lowercase();
            if lowered != "true" && lowered != "false" {
                eprintln!(
                    "codex-cli config: remote-refresh must be true|false (got: {})",
                    value
                );
                return 64;
            }
            println!("export {}={}", remote::ENV_AUTH_REMOTE_REFRESH, lowered);
            0
        }
        _ => {
            eprintln!("codex-cli config: unknown key: {key}");
            eprintln!(
                "codex-cli config: keys: model|reasoning|agent-runtime|ephemeral|dangerous|remote-ssh|remote-name|remote-refresh"
            );
            64
        }
    }
}
