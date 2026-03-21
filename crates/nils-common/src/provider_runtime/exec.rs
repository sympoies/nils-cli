use crate::env as shared_env;
use crate::process as shared_process;
use std::io::Write;
use std::sync::atomic::Ordering;

use super::error::CoreError;
use super::profile::{ExecInvocation, ProviderProfile};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ExecOptions {
    pub ephemeral: bool,
}

pub fn require_allow_dangerous(
    profile: &ProviderProfile,
    caller: Option<&str>,
    stderr: &mut impl Write,
) -> bool {
    if is_true_env(profile, stderr) {
        return true;
    }

    let prefix = match caller {
        Some(value) if !value.is_empty() => value,
        _ => profile.exec.default_caller_prefix,
    };
    let _ = writeln!(
        stderr,
        "{prefix}: disabled (set {}=true)",
        profile.env.allow_dangerous_enabled,
    );
    false
}

pub fn allow_dangerous_status(
    profile: &ProviderProfile,
    caller: Option<&str>,
) -> (bool, Option<String>) {
    let mut stderr = Vec::new();
    let enabled = require_allow_dangerous(profile, caller, &mut stderr);
    let text = String::from_utf8_lossy(&stderr).trim_end().to_string();
    (enabled, if text.is_empty() { None } else { Some(text) })
}

pub fn check_allow_dangerous(
    profile: &ProviderProfile,
    caller: Option<&str>,
) -> Result<(), CoreError> {
    let (enabled, message) = allow_dangerous_status(profile, caller);
    if enabled {
        return Ok(());
    }
    Err(CoreError::validation(
        "disabled-policy",
        message.unwrap_or_else(|| {
            format!(
                "execution disabled (set {}=true)",
                profile.env.allow_dangerous_enabled,
            )
        }),
    )
    .with_retryable(false))
}

pub fn exec_dangerous(
    profile: &ProviderProfile,
    prompt: &str,
    caller: &str,
    stderr: &mut impl Write,
) -> i32 {
    exec_dangerous_with_options(profile, prompt, caller, stderr, ExecOptions::default())
}

pub fn exec_dangerous_with_options(
    profile: &ProviderProfile,
    prompt: &str,
    caller: &str,
    stderr: &mut impl Write,
    options: ExecOptions,
) -> i32 {
    if prompt.is_empty() {
        let _ = writeln!(
            stderr,
            "{}: missing prompt",
            profile.exec.missing_prompt_label
        );
        return 1;
    }

    if !require_allow_dangerous(profile, Some(caller), stderr) {
        return 1;
    }

    match profile.exec.invocation {
        ExecInvocation::CodexStyle => exec_dangerous_codex_style(profile, prompt, stderr, options),
        ExecInvocation::GeminiStyle => exec_dangerous_gemini_style(profile, prompt, stderr),
    }
}

fn exec_dangerous_codex_style(
    profile: &ProviderProfile,
    prompt: &str,
    stderr: &mut impl Write,
    options: ExecOptions,
) -> i32 {
    let model = shared_env::env_or_default(profile.env.model, profile.defaults.model);
    let reasoning = shared_env::env_or_default(profile.env.reasoning, profile.defaults.reasoning);
    let reasoning_arg = format!("model_reasoning_effort=\"{}\"", reasoning);
    let mut args = vec![
        "exec".to_string(),
        "--dangerously-bypass-approvals-and-sandbox".to_string(),
        "-s".to_string(),
        "workspace-write".to_string(),
        "-m".to_string(),
        model,
        "-c".to_string(),
        reasoning_arg,
    ];
    if options.ephemeral {
        args.push("--ephemeral".to_string());
    }
    args.push("--".to_string());
    args.push(prompt.to_string());

    let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    run_exec(profile, &arg_refs, stderr)
}

fn exec_dangerous_gemini_style(
    profile: &ProviderProfile,
    prompt: &str,
    stderr: &mut impl Write,
) -> i32 {
    let model = shared_env::env_or_default(profile.env.model, profile.defaults.model);
    let prompt_arg = format!("--prompt={prompt}");
    let args = [
        prompt_arg.as_str(),
        "--model",
        model.as_str(),
        "--approval-mode",
        "yolo",
    ];

    run_exec(profile, &args, stderr)
}

fn run_exec(profile: &ProviderProfile, args: &[&str], stderr: &mut impl Write) -> i32 {
    match shared_process::run_status_inherit(profile.exec.binary_name, args) {
        Ok(status) => status.code().unwrap_or(1),
        Err(err) => {
            let _ = writeln!(stderr, "{}: {err}", profile.exec.failed_exec_message_prefix,);
            1
        }
    }
}

fn is_true_env(profile: &ProviderProfile, stderr: &mut impl Write) -> bool {
    let key = profile.env.allow_dangerous_enabled;
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
            if !profile
                .exec
                .warned_invalid_allow_dangerous
                .swap(true, Ordering::SeqCst)
            {
                let _ = writeln!(
                    stderr,
                    "warning: {key} must be true|false (got: {raw}); treating as false"
                );
            }
            false
        }
    }
}
