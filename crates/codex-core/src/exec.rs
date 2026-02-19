use nils_common::process as shared_process;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::config::{DEFAULT_MODEL, DEFAULT_REASONING};
use crate::error::CoreError;

static WARNED_INVALID_ALLOW_DANGEROUS: AtomicBool = AtomicBool::new(false);

pub fn require_allow_dangerous(caller: Option<&str>, stderr: &mut impl Write) -> bool {
    if is_true_env("CODEX_ALLOW_DANGEROUS_ENABLED", stderr) {
        return true;
    }

    let prefix = match caller {
        Some(value) if !value.is_empty() => value,
        _ => "codex",
    };
    let _ = writeln!(
        stderr,
        "{prefix}: disabled (set CODEX_ALLOW_DANGEROUS_ENABLED=true)"
    );
    false
}

pub fn allow_dangerous_status(caller: Option<&str>) -> (bool, Option<String>) {
    let mut stderr = Vec::new();
    let enabled = require_allow_dangerous(caller, &mut stderr);
    let text = String::from_utf8_lossy(&stderr).trim_end().to_string();
    (enabled, if text.is_empty() { None } else { Some(text) })
}

pub fn check_allow_dangerous(caller: Option<&str>) -> Result<(), CoreError> {
    let (enabled, message) = allow_dangerous_status(caller);
    if enabled {
        return Ok(());
    }
    Err(CoreError::validation(
        "disabled-policy",
        message.unwrap_or_else(|| {
            "execution disabled (set CODEX_ALLOW_DANGEROUS_ENABLED=true)".to_string()
        }),
    )
    .with_retryable(false))
}

pub fn exec_dangerous(prompt: &str, caller: &str, stderr: &mut impl Write) -> i32 {
    if prompt.is_empty() {
        let _ = writeln!(stderr, "_codex_exec_dangerous: missing prompt");
        return 1;
    }

    if !require_allow_dangerous(Some(caller), stderr) {
        return 1;
    }

    let model = env_or_default("CODEX_CLI_MODEL", DEFAULT_MODEL);
    let reasoning = env_or_default("CODEX_CLI_REASONING", DEFAULT_REASONING);
    let reasoning_arg = format!("model_reasoning_effort=\"{}\"", reasoning);
    let args = [
        "exec",
        "--dangerously-bypass-approvals-and-sandbox",
        "-s",
        "workspace-write",
        "-m",
        model.as_str(),
        "-c",
        reasoning_arg.as_str(),
        "--",
        prompt,
    ];

    match shared_process::run_status_inherit("codex", &args) {
        Ok(status) => status.code().unwrap_or(1),
        Err(err) => {
            let _ = writeln!(stderr, "codex-tools: failed to run codex exec: {err}");
            1
        }
    }
}

fn env_or_default(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

fn is_true_env(key: &str, stderr: &mut impl Write) -> bool {
    let Ok(raw) = std::env::var(key) else {
        return false;
    };

    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return false;
    }

    match trimmed.to_ascii_lowercase().as_str() {
        "true" => true,
        "false" => false,
        _ => {
            if !WARNED_INVALID_ALLOW_DANGEROUS.swap(true, Ordering::SeqCst) {
                let _ = writeln!(
                    stderr,
                    "warning: {key} must be true|false (got: {raw}); treating as false"
                );
            }
            false
        }
    }
}
