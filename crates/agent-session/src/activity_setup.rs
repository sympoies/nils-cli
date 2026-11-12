use super::*;

pub(super) fn forward_activity_setup_to_agent_hook(
    args: &cli::ActivitySetupArgs,
) -> Result<Value, CliError> {
    if args.agent == AgentKind::Dsh {
        return Err(CliError::usage(
            "unsupported-activity-agent",
            "dsh activity comes from the dsh-runtime-kit bridge hook installed by the launch profile, or from the external runtime's liveness sidecar; there is no provider activity configuration to manage",
            Some(json!({ "agent": args.agent.as_str() })),
        ));
    }
    let action = if args.dry_run {
        "--dry-run"
    } else if args.apply {
        "--apply"
    } else if args.remove {
        "--remove"
    } else {
        "--repair"
    };
    let mut command = ProcessCommand::new("/bin/sh");
    command.args([
        "-c",
        &nils_common::provider_entrypoint::selector(true),
        "provider-entrypoint",
    ]);
    // The hook that writes the provider config renders the session binary name
    // it was built with. Unless the caller pins `AGENT_SESSION_BIN`, the selector
    // takes the hook beside this executable (the same release) before any trusted
    // root, provided it is owner-controlled. Without such a hook the variable
    // stays unset and the trusted roots apply; an executable
    // not running under the configured name also falls through to them.
    if std::env::var_os("AGENT_SESSION_BIN").is_none_or(|value| value.is_empty())
        && let Ok(current) = std::env::current_exe()
        && sibling_hook_is_trusted(&current)
    {
        command.env("AGENT_SESSION_BIN", current);
    }
    command.args([
        "setup",
        "--product",
        args.agent.as_str(),
        action,
        "--format",
        "json",
    ]);
    if let Some(digest) = args.expected_preview_digest.as_deref() {
        command.args(["--expected-plan-digest", digest]);
    }
    let output = match run_output_with_timeout_and_cap(
        command,
        Duration::from_secs(10),
        AGENT_HOOK_SETUP_MAX_OUTPUT_BYTES,
    ) {
        Ok(output) => output,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(CliError::unavailable(
                "agent-hook-setup-unavailable",
                "agent-hook is required for provider setup; install the matching nils-agent-hook binary, then rerun the same dry-run before any apply, repair, or remove",
                Some(json!({
                    "compatibility_owner": "agent-hook",
                    "action": "install-agent-hook-and-repeat-reviewed-preview"
                })),
            ));
        }
        Err(error) => {
            return Err(CliError::unavailable(
                "agent-hook-setup-unavailable",
                format!("agent-hook setup compatibility forward failed: {error}"),
                Some(json!({
                    "compatibility_owner": "agent-hook",
                    "action": "inspect-agent-hook-installation-and-repeat-reviewed-preview"
                })),
            ));
        }
    };
    if output.status.code() == Some(127) {
        return Err(CliError::unavailable(
            "agent-hook-setup-unavailable",
            "No selectable agent-hook in the trusted toolchain; install the matching nils-agent-hook binary, then rerun the same dry-run before any apply, repair, or remove",
            Some(
                json!({ "compatibility_owner": "agent-hook", "action": "install-agent-hook-and-repeat-reviewed-preview" }),
            ),
        ));
    }
    let envelope: Envelope<Value> = match serde_json::from_slice(&output.stdout) {
        Ok(value) => value,
        Err(_) => {
            return Err(CliError::data(
                "agent-hook-setup-output-invalid",
                "agent-hook setup returned invalid JSON",
                None,
            ));
        }
    };
    if envelope.schema_version != "cli.agent-hook.setup.v1" {
        return Err(CliError::data(
            "agent-hook-setup-output-invalid",
            "agent-hook setup returned an unsupported envelope schema",
            None,
        ));
    }
    if !output.status.success() || !envelope.ok {
        let (code, message, details) = envelope.error.map_or_else(
            || {
                (
                    "agent-hook-setup-failed".to_string(),
                    "agent-hook setup rejected the compatibility request".to_string(),
                    None,
                )
            },
            |error| (error.code, error.message, error.details),
        );
        let child_exit = output
            .status
            .code()
            .filter(|code| {
                matches!(
                    *code,
                    exit::RUNTIME | exit::USAGE | exit::DATA | exit::UNAVAILABLE | exit::SOFTWARE
                )
            })
            .unwrap_or(exit::DATA);
        return Err(CliError::with_exit_code(code, message, details, child_exit));
    }
    let typed: Envelope<agent_hook::setup::SetupResult> = serde_json::from_slice(&output.stdout)
        .map_err(|_| {
            CliError::data(
                "agent-hook-setup-output-invalid",
                "agent-hook setup returned an incomplete success contract",
                None,
            )
        })?;
    let result = typed.data.ok_or_else(|| {
        CliError::data(
            "agent-hook-setup-output-invalid",
            "agent-hook setup omitted its data object",
            None,
        )
    })?;
    if result.schema_version != "agent-hook.setup-result.v1"
        || result.product != args.agent.as_str()
        || result.action != action.trim_start_matches("--")
        || result.compatibility_owner != "agent-hook"
        || !valid_sha256(&result.plan_digest)
        || !valid_sha256(&result.config_digest)
        || !valid_sha256(&result.policy_digest)
    {
        return Err(CliError::data(
            "agent-hook-setup-output-invalid",
            "agent-hook setup response does not match the requested operation",
            None,
        ));
    }
    let mut result = serde_json::to_value(result)
        .ok()
        .and_then(|value| value.as_object().cloned())
        .ok_or_else(|| {
            CliError::data(
                "agent-hook-setup-output-invalid",
                "agent-hook setup result could not be projected",
                None,
            )
        })?;
    result.insert(
        "compatibility_owner".to_string(),
        Value::String("agent-hook".to_string()),
    );
    Ok(Value::Object(result))
}

/// The same owner and mode rule `session_binary::trusted_root` applies to the
/// session binary: an executable regular file owned by the caller that no
/// other user can write.
fn sibling_hook_is_trusted(current: &std::path::Path) -> bool {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    current
        .with_file_name("agent-hook")
        .metadata()
        .is_ok_and(|metadata| {
            metadata.is_file()
                && metadata.uid() == unsafe { libc::geteuid() }
                && metadata.permissions().mode() & 0o022 == 0
                && metadata.permissions().mode() & 0o111 != 0
        })
}
