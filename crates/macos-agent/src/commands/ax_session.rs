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
