use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use std::time::Duration;

use nils_common::fs::home_dir;
use serde_json::Value;

use super::{AgentKind, CliError};
use crate::{Envelope, run_output_with_timeout_and_strict_cap, valid_sha256};

const AGENT_HOOK_DOCTOR_MAX_OUTPUT_BYTES: usize = 1024 * 1024;
const AGENT_HOOK_DISPATCH_TIMEOUT_SECONDS: i64 = 60;

pub(super) struct ClaudeRegistrationReadiness {
    pub(super) configured: bool,
    pub(super) missing_owned_events: Vec<String>,
}

pub(super) fn provider_config_path(agent: AgentKind) -> Result<PathBuf, CliError> {
    let home = home_dir().ok_or_else(|| {
        CliError::runtime(
            "home-unavailable",
            "HOME is required for provider activity setup",
            None,
        )
    })?;
    Ok(match agent {
        AgentKind::Codex => config_dir("CODEX_HOME", home.join(".codex")).join("hooks.json"),
        AgentKind::Claude => {
            config_dir("CLAUDE_CONFIG_DIR", home.join(".claude")).join("settings.json")
        }
        AgentKind::Dsh => {
            return Err(CliError::usage(
                "unsupported-activity-agent",
                "dsh activity comes from the dsh-runtime-kit bridge hook installed by the launch profile, or from the external runtime's liveness sidecar; there is no provider activity configuration to manage",
                Some(serde_json::json!({ "agent": agent.as_str() })),
            ));
        }
    })
}

pub(super) fn codex_notification_config_path() -> Result<PathBuf, CliError> {
    let home = home_dir().ok_or_else(|| {
        CliError::runtime(
            "home-unavailable",
            "HOME is required for provider activity setup",
            None,
        )
    })?;
    Ok(config_dir("CODEX_HOME", home.join(".codex")).join("config.toml"))
}

fn config_dir(variable: &str, fallback: PathBuf) -> PathBuf {
    std::env::var_os(variable)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or(fallback)
}

pub(super) fn agent_hook_codex_configured() -> Result<bool, &'static str> {
    let binary = std::env::var_os("AGENT_HOOK_BIN").unwrap_or_else(|| "agent-hook".into());
    let mut command = ProcessCommand::new(binary);
    command.args(["doctor", "--product", "codex", "--format", "json"]);
    let output = run_output_with_timeout_and_strict_cap(
        command,
        Duration::from_secs(10),
        AGENT_HOOK_DOCTOR_MAX_OUTPUT_BYTES,
    )
    .map_err(|error| {
        if error.kind() == io::ErrorKind::InvalidData {
            "agent-hook-doctor-output-invalid"
        } else {
            "agent-hook-doctor-unavailable"
        }
    })?;
    let envelope: Envelope<Vec<agent_hook::setup::DoctorResult>> =
        serde_json::from_slice(&output.stdout).map_err(|_| "agent-hook-doctor-output-invalid")?;
    if !output.status.success()
        || !envelope.ok
        || envelope.error.is_some()
        || envelope.schema_version != "cli.agent-hook.doctor.v1"
    {
        return Err("agent-hook-doctor-output-invalid");
    }
    let mut results = envelope
        .data
        .ok_or("agent-hook-doctor-output-invalid")?
        .into_iter();
    let result = results.next().ok_or("agent-hook-doctor-output-invalid")?;
    if results.next().is_some()
        || result.schema_version != "agent-hook.doctor.v1"
        || result.product != "codex"
        || !valid_sha256(&result.config_digest)
        || !valid_sha256(&result.policy_digest)
    {
        return Err("agent-hook-doctor-output-invalid");
    }
    Ok(result.supported
        && result.status == agent_hook::setup::ProviderStatus::Converged
        && result.owned_count == result.expected_owned_count
        && result.legacy_residue_count == 0)
}

pub(super) fn agent_hook_claude_readiness(
    path: &Path,
) -> Result<ClaudeRegistrationReadiness, &'static str> {
    let binary = std::env::var_os("AGENT_HOOK_BIN").unwrap_or_else(|| "agent-hook".into());
    let mut command = ProcessCommand::new(binary);
    command.args([
        "setup",
        "--product",
        "claude",
        "--dry-run",
        "--format",
        "json",
    ]);
    let output = run_output_with_timeout_and_strict_cap(
        command,
        Duration::from_secs(10),
        AGENT_HOOK_DOCTOR_MAX_OUTPUT_BYTES,
    )
    .map_err(|error| {
        if error.kind() == io::ErrorKind::InvalidData {
            "agent-hook-setup-output-invalid"
        } else {
            "agent-hook-setup-unavailable"
        }
    })?;
    let envelope: Envelope<agent_hook::setup::SetupResult> =
        serde_json::from_slice(&output.stdout).map_err(|_| "agent-hook-setup-output-invalid")?;
    if !output.status.success()
        || !envelope.ok
        || envelope.error.is_some()
        || envelope.schema_version != "cli.agent-hook.setup.v1"
    {
        return Err("agent-hook-setup-output-invalid");
    }
    let result = envelope.data.ok_or("agent-hook-setup-output-invalid")?;
    if result.schema_version != "agent-hook.setup-result.v1"
        || result.product != "claude"
        || result.action != "dry-run"
        || !valid_sha256(&result.config_digest)
        || !valid_sha256(&result.policy_digest)
        || result.owned_groups.is_empty()
    {
        return Err("agent-hook-setup-output-invalid");
    }

    let config = fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .unwrap_or(Value::Null);
    let mut missing_owned_events = Vec::new();
    for group in &result.owned_groups {
        if !has_dispatch_group(&config, group.event.as_str(), group.matcher.as_deref())
            && !missing_owned_events.contains(&group.event)
        {
            missing_owned_events.push(group.event.clone());
        }
    }
    let configured = result.configured
        && result.status == agent_hook::setup::ProviderStatus::Converged
        && result.owned_count == result.owned_groups.len()
        && result.legacy_residue_count == 0
        && missing_owned_events.is_empty();
    Ok(ClaudeRegistrationReadiness {
        configured,
        missing_owned_events,
    })
}

fn has_dispatch_group(config: &Value, event: &str, matcher: Option<&str>) -> bool {
    config
        .get("hooks")
        .and_then(|hooks| hooks.get(event))
        .and_then(Value::as_array)
        .is_some_and(|groups| {
            groups.iter().any(|group| {
                group.get("matcher").and_then(Value::as_str) == matcher
                    && group
                        .get("hooks")
                        .and_then(Value::as_array)
                        .is_some_and(|handlers| handlers.iter().any(is_owned_dispatch))
            })
        })
}

fn is_owned_dispatch(handler: &Value) -> bool {
    handler.as_object().is_some_and(|object| object.len() == 3)
        && handler.get("type").and_then(Value::as_str) == Some("command")
        && handler.get("command").and_then(Value::as_str)
            == Some("agent-hook dispatch --product claude")
        && handler.get("timeout").and_then(Value::as_i64)
            == Some(AGENT_HOOK_DISPATCH_TIMEOUT_SECONDS)
}
