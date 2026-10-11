use super::*;

pub(super) fn add_runtime_tmux_environment(
    command: &mut ProcessCommand,
    state_dir: &Path,
    record: &SessionRecord,
) -> Result<(), CliError> {
    for (key, value) in launch_env::from_record(record)? {
        command.arg("-e").arg(format!("{key}={value}"));
    }
    let runtime_id = record
        .runtime
        .as_ref()
        .map(|runtime| runtime.launch_id.as_str())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            CliError::data(
                "runtime-id-missing",
                "session runtime is missing its launch id",
                Some(json!({ "id": record.id })),
            )
        })?;
    for key in [
        "AGENT_SESSION_TOKEN",
        "AGENT_SESSION_RELAY_URL",
        "AGENT_SESSION_RELAY_TOKEN",
        "AGENT_SESSION_RELAY_INGRESS_TOKEN",
        "AGENT_CONSOLE_COORDINATION_RELAYS",
    ] {
        command.env_remove(key).arg("-e").arg(key);
    }
    for value in [
        format!("AGENT_SESSION_ID={}", record.id),
        format!("AGENT_SESSION_STATE_DIR={}", display_path(state_dir)),
        format!("AGENT_SESSION_RUNTIME_ID={runtime_id}"),
        format!(
            "AGENT_SESSION_COORDINATION_MODE={}",
            record.coordination_mode.as_str()
        ),
        format!(
            "{}={}",
            coordination::CAPABILITY_ENV,
            display_path(&coordination::capability_path_for_state(
                state_dir, &record.id, runtime_id
            ))
        ),
        format!(
            "{}={}",
            coordination::CHECKPOINT_ENV,
            display_path(&coordination::checkpoint_path_for_state(
                state_dir, &record.id, runtime_id
            ))
        ),
        format!(
            "{}={}",
            codex_app_server::ATTENTION_AUTHORITY_ENV,
            codex_app_server::attention_authority(record)
        ),
    ] {
        command.arg("-e").arg(value);
    }

    if let (Some(agent), Some(config_dir)) = (
        AgentKind::from_name(&record.agent),
        session_effective_provider_config_dir(record),
    ) {
        let env_key = match agent {
            AgentKind::Codex => Some("CODEX_HOME"),
            AgentKind::Claude => Some("CLAUDE_CONFIG_DIR"),
            AgentKind::Dsh => None,
        };
        if let Some(env_key) = env_key {
            let mut assignment = OsString::from(format!("{env_key}="));
            assignment.push(config_dir);
            command.arg("-e").arg(assignment);
        }
    }
    if AgentKind::from_name(&record.agent) == Some(AgentKind::Claude) {
        // Managed panes are attached as plain terminals whose scrollback the
        // client owns, so Claude stays on its main-screen renderer just as
        // Codex gets --no-alt-screen. Without this, Claude Code picks its
        // alt-screen renderer on fresh installs or rollout gates, and touch
        // scrolling becomes a wheel round trip into its virtual viewport.
        command
            .arg("-e")
            .arg("CLAUDE_CODE_DISABLE_ALTERNATE_SCREEN=1");
    }
    if let Some(path) = env::var_os("PATH") {
        // A long-lived tmux server keeps the environment from when that server
        // started. Pin each new session to the current daemon PATH so provider
        // hooks cannot resolve an older agent-session helper after a staged
        // daemon upgrade.
        let mut assignment = OsString::from("PATH=");
        assignment.push(path);
        command.arg("-e").arg(assignment);
    }
    // Pinning PATH alone is not enough: AGENT_SESSION_BIN outranks PATH when
    // agent-hook resolves the activity helper, so a value cached by the tmux
    // server survives the PATH pin and fail-closes every UserPromptSubmit and
    // Stop in a session created long after the helper moved. Pin the executable
    // that is launching this session, which `HELD_LAUNCH_SCRIPT` already binds
    // as `broker_bin` for the session's whole life, so this adds no lifetime
    // coupling the runtime does not already have. See sympoies/nils-cli#1414.
    if let Ok(helper) = resolve_agent_session_executable() {
        let mut assignment = OsString::from("AGENT_SESSION_BIN=");
        assignment.push(helper);
        command.arg("-e").arg(assignment);
    }
    Ok(())
}
