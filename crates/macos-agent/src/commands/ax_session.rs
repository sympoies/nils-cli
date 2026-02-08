use crate::backend::process::ProcessRunner;
use crate::backend::{AutoAxBackend, AxBackendAdapter};
use crate::cli::{AxSessionListArgs, AxSessionStartArgs, AxSessionStopArgs, OutputFormat};
use crate::commands::ax_common::build_target;
use crate::error::CliError;
use crate::model::{
    AxSessionListResult, AxSessionStartRequest, AxSessionStartResult, AxSessionStopRequest,
    AxSessionStopResult, SuccessEnvelope,
};
use crate::run::ActionPolicy;

pub fn run_start(
    format: OutputFormat,
    args: &AxSessionStartArgs,
    policy: ActionPolicy,
    runner: &dyn ProcessRunner,
) -> Result<(), CliError> {
    let request = AxSessionStartRequest {
        target: build_target(
            None,
            args.app.clone(),
            args.bundle_id.clone(),
            args.window_title_contains.clone(),
        )?,
        session_id: args.session_id.clone(),
    };

    let result = if policy.dry_run {
        AxSessionStartResult {
            session: crate::model::AxSessionInfo {
                session_id: request
                    .session_id
                    .clone()
                    .unwrap_or_else(|| "axs-dry-run".to_string()),
                app: request.target.app.clone(),
                bundle_id: request.target.bundle_id.clone(),
                pid: None,
                window_title_contains: request.target.window_title_contains.clone(),
                created_at_ms: 0,
            },
            created: false,
        }
    } else {
        let backend = AutoAxBackend::default();
        backend.session_start(runner, &request, policy.timeout_ms)?
    };

    print_start(format, result)
}

pub fn run_list(
    format: OutputFormat,
    _args: &AxSessionListArgs,
    policy: ActionPolicy,
    runner: &dyn ProcessRunner,
) -> Result<(), CliError> {
    let backend = AutoAxBackend::default();
    let result: AxSessionListResult = backend.session_list(runner, policy.timeout_ms)?;

    match format {
        OutputFormat::Json => {
            let payload = SuccessEnvelope::new("ax.session.list", result);
            println!(
                "{}",
                serde_json::to_string(&payload).map_err(|err| CliError::runtime(format!(
                    "failed to serialize json output: {err}"
                )))?
            );
        }
        OutputFormat::Text => {
            if result.sessions.is_empty() {
                println!("ax.session.list\tsessions=0");
            } else {
                for session in result.sessions {
                    println!(
                        "ax.session.list\tsession_id={}\tapp={}\tbundle_id={}\tpid={}\tcreated_at_ms={}",
                        session.session_id,
                        session.app.unwrap_or_default(),
                        session.bundle_id.unwrap_or_default(),
                        session.pid.unwrap_or_default(),
                        session.created_at_ms,
                    );
                }
            }
        }
        OutputFormat::Tsv => {
            return Err(CliError::usage(
                "--format tsv is only supported for `windows list` and `apps list`",
            ));
        }
    }

    Ok(())
}

pub fn run_stop(
    format: OutputFormat,
    args: &AxSessionStopArgs,
    policy: ActionPolicy,
    runner: &dyn ProcessRunner,
) -> Result<(), CliError> {
    let request = AxSessionStopRequest {
        session_id: args.session_id.clone(),
    };

    let result = if policy.dry_run {
        AxSessionStopResult {
            session_id: request.session_id,
            removed: false,
        }
    } else {
        let backend = AutoAxBackend::default();
        backend.session_stop(runner, &request, policy.timeout_ms)?
    };

    match format {
        OutputFormat::Json => {
            let payload = SuccessEnvelope::new("ax.session.stop", result);
            println!(
                "{}",
                serde_json::to_string(&payload).map_err(|err| CliError::runtime(format!(
                    "failed to serialize json output: {err}"
                )))?
            );
        }
        OutputFormat::Text => {
            println!(
                "ax.session.stop\tsession_id={}\tremoved={}",
                result.session_id, result.removed
            );
        }
        OutputFormat::Tsv => {
            return Err(CliError::usage(
                "--format tsv is only supported for `windows list` and `apps list`",
            ));
        }
    }

    Ok(())
}

fn print_start(format: OutputFormat, result: AxSessionStartResult) -> Result<(), CliError> {
    match format {
        OutputFormat::Json => {
            let payload = SuccessEnvelope::new("ax.session.start", result);
            println!(
                "{}",
                serde_json::to_string(&payload).map_err(|err| CliError::runtime(format!(
                    "failed to serialize json output: {err}"
                )))?
            );
        }
        OutputFormat::Text => {
            println!(
                "ax.session.start\tsession_id={}\tapp={}\tbundle_id={}\tpid={}\tcreated={}\tcreated_at_ms={}",
                result.session.session_id,
                result.session.app.unwrap_or_default(),
                result.session.bundle_id.unwrap_or_default(),
                result.session.pid.unwrap_or_default(),
                result.created,
                result.session.created_at_ms,
            );
        }
        OutputFormat::Tsv => {
            return Err(CliError::usage(
                "--format tsv is only supported for `windows list` and `apps list`",
            ));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use nils_test_support::{EnvGuard, GlobalStateLock};

    use super::{run_list, run_start, run_stop};
    use crate::backend::process::RealProcessRunner;
    use crate::cli::{AxSessionListArgs, AxSessionStartArgs, AxSessionStopArgs, OutputFormat};
    use crate::run::ActionPolicy;

    fn policy(dry_run: bool) -> ActionPolicy {
        ActionPolicy {
            dry_run,
            retries: 0,
            retry_delay_ms: 150,
            timeout_ms: 1000,
        }
    }

    fn sample_start_args() -> AxSessionStartArgs {
        AxSessionStartArgs {
            app: Some("Arc".to_string()),
            bundle_id: None,
            session_id: Some("axs-unit".to_string()),
            window_title_contains: Some("Inbox".to_string()),
        }
    }

    fn sample_stop_args() -> AxSessionStopArgs {
        AxSessionStopArgs {
            session_id: "axs-unit".to_string(),
        }
    }

    #[test]
    fn run_start_and_stop_dry_run_support_text_and_json() {
        let lock = GlobalStateLock::new();
        let _mode = EnvGuard::set(&lock, "CODEX_MACOS_AGENT_TEST_MODE", "1");
        let runner = RealProcessRunner;

        run_start(
            OutputFormat::Text,
            &sample_start_args(),
            policy(true),
            &runner,
        )
        .expect("start text dry-run should succeed");
        run_start(
            OutputFormat::Json,
            &sample_start_args(),
            policy(true),
            &runner,
        )
        .expect("start json dry-run should succeed");

        run_stop(
            OutputFormat::Text,
            &sample_stop_args(),
            policy(true),
            &runner,
        )
        .expect("stop text dry-run should succeed");
        run_stop(
            OutputFormat::Json,
            &sample_stop_args(),
            policy(true),
            &runner,
        )
        .expect("stop json dry-run should succeed");
    }

    #[test]
    fn run_start_and_stop_reject_tsv_in_dry_run() {
        let lock = GlobalStateLock::new();
        let _mode = EnvGuard::set(&lock, "CODEX_MACOS_AGENT_TEST_MODE", "1");
        let runner = RealProcessRunner;

        let start_err = run_start(
            OutputFormat::Tsv,
            &sample_start_args(),
            policy(true),
            &runner,
        )
        .expect_err("start tsv should be rejected");
        assert!(start_err.to_string().contains("windows list"));

        let stop_err = run_stop(
            OutputFormat::Tsv,
            &sample_stop_args(),
            policy(true),
            &runner,
        )
        .expect_err("stop tsv should be rejected");
        assert!(stop_err.to_string().contains("windows list"));
    }

    #[test]
    fn run_list_covers_non_empty_text_and_tsv_rejection() {
        let lock = GlobalStateLock::new();
        let _mode = EnvGuard::set(&lock, "CODEX_MACOS_AGENT_TEST_MODE", "1");
        let _backend = EnvGuard::set(&lock, "CODEX_MACOS_AGENT_AX_BACKEND", "hammerspoon");
        let _list_override = EnvGuard::set(
            &lock,
            "CODEX_MACOS_AGENT_AX_SESSION_LIST_JSON",
            r#"{"sessions":[{"session_id":"axs-unit","app":"Arc","bundle_id":"company.thebrowser.Browser","pid":4242,"window_title_contains":"Inbox","created_at_ms":1700000001000}]}"#,
        );
        let runner = RealProcessRunner;

        run_list(
            OutputFormat::Text,
            &AxSessionListArgs::default(),
            policy(false),
            &runner,
        )
        .expect("list text should succeed");

        let err = run_list(
            OutputFormat::Tsv,
            &AxSessionListArgs::default(),
            policy(false),
            &runner,
        )
        .expect_err("list tsv should be rejected");
        assert!(err.to_string().contains("windows list"));
    }

    #[test]
    fn run_list_text_supports_empty_sessions_branch() {
        let lock = GlobalStateLock::new();
        let _mode = EnvGuard::set(&lock, "CODEX_MACOS_AGENT_TEST_MODE", "1");
        let _backend = EnvGuard::set(&lock, "CODEX_MACOS_AGENT_AX_BACKEND", "hammerspoon");
        let _list_override = EnvGuard::set(
            &lock,
            "CODEX_MACOS_AGENT_AX_SESSION_LIST_JSON",
            r#"{"sessions":[]}"#,
        );
        let runner = RealProcessRunner;

        run_list(
            OutputFormat::Text,
            &AxSessionListArgs::default(),
            policy(false),
            &runner,
        )
        .expect("list text should succeed with empty sessions");
    }
}
