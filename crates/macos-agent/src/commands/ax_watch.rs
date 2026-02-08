use crate::backend::process::ProcessRunner;
use crate::backend::{AutoAxBackend, AxBackendAdapter};
use crate::cli::{AxWatchPollArgs, AxWatchStartArgs, AxWatchStopArgs, OutputFormat};
use crate::error::CliError;
use crate::model::{
    AxWatchPollRequest, AxWatchPollResult, AxWatchStartRequest, AxWatchStartResult,
    AxWatchStopRequest, AxWatchStopResult, SuccessEnvelope,
};
use crate::run::ActionPolicy;

pub fn run_start(
    format: OutputFormat,
    args: &AxWatchStartArgs,
    policy: ActionPolicy,
    runner: &dyn ProcessRunner,
) -> Result<(), CliError> {
    let request = AxWatchStartRequest {
        session_id: args.session_id.clone(),
        events: args.events.clone(),
        max_buffer: args.max_buffer,
        watch_id: args.watch_id.clone(),
    };

    let result = if policy.dry_run {
        AxWatchStartResult {
            watch_id: request
                .watch_id
                .clone()
                .unwrap_or_else(|| "axw-dry-run".to_string()),
            session_id: request.session_id,
            events: request.events,
            max_buffer: request.max_buffer,
            started: false,
        }
    } else {
        let backend = AutoAxBackend::default();
        backend.watch_start(runner, &request, policy.timeout_ms)?
    };

    match format {
        OutputFormat::Json => {
            let payload = SuccessEnvelope::new("ax.watch.start", result);
            println!(
                "{}",
                serde_json::to_string(&payload).map_err(|err| CliError::runtime(format!(
                    "failed to serialize json output: {err}"
                )))?
            );
        }
        OutputFormat::Text => {
            println!(
                "ax.watch.start\twatch_id={}\tsession_id={}\tstarted={}\tevents={}\tmax_buffer={}",
                result.watch_id,
                result.session_id,
                result.started,
                result.events.join(","),
                result.max_buffer
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

pub fn run_poll(
    format: OutputFormat,
    args: &AxWatchPollArgs,
    policy: ActionPolicy,
    runner: &dyn ProcessRunner,
) -> Result<(), CliError> {
    let request = AxWatchPollRequest {
        watch_id: args.watch_id.clone(),
        limit: args.limit,
        drain: args.drain,
    };

    let backend = AutoAxBackend::default();
    let result: AxWatchPollResult = backend.watch_poll(runner, &request, policy.timeout_ms)?;

    match format {
        OutputFormat::Json => {
            let payload = SuccessEnvelope::new("ax.watch.poll", result);
            println!(
                "{}",
                serde_json::to_string(&payload).map_err(|err| CliError::runtime(format!(
                    "failed to serialize json output: {err}"
                )))?
            );
        }
        OutputFormat::Text => {
            println!(
                "ax.watch.poll\twatch_id={}\tevents={}\tdropped={}\trunning={}",
                result.watch_id,
                result.events.len(),
                result.dropped,
                result.running,
            );
            for event in result.events {
                println!(
                    "ax.watch.event\twatch_id={}\tevent={}\tat_ms={}\trole={}\ttitle={}",
                    event.watch_id,
                    event.event,
                    event.at_ms,
                    event.role.unwrap_or_default(),
                    event.title.unwrap_or_default()
                );
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
    args: &AxWatchStopArgs,
    policy: ActionPolicy,
    runner: &dyn ProcessRunner,
) -> Result<(), CliError> {
    let request = AxWatchStopRequest {
        watch_id: args.watch_id.clone(),
    };

    let result = if policy.dry_run {
        AxWatchStopResult {
            watch_id: request.watch_id,
            stopped: false,
            drained: 0,
        }
    } else {
        let backend = AutoAxBackend::default();
        backend.watch_stop(runner, &request, policy.timeout_ms)?
    };

    match format {
        OutputFormat::Json => {
            let payload = SuccessEnvelope::new("ax.watch.stop", result);
            println!(
                "{}",
                serde_json::to_string(&payload).map_err(|err| CliError::runtime(format!(
                    "failed to serialize json output: {err}"
                )))?
            );
        }
        OutputFormat::Text => {
            println!(
                "ax.watch.stop\twatch_id={}\tstopped={}\tdrained={}",
                result.watch_id, result.stopped, result.drained
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

    use super::{run_poll, run_start, run_stop};
    use crate::backend::process::RealProcessRunner;
    use crate::cli::{AxWatchPollArgs, AxWatchStartArgs, AxWatchStopArgs, OutputFormat};
    use crate::run::ActionPolicy;

    fn policy(dry_run: bool) -> ActionPolicy {
        ActionPolicy {
            dry_run,
            retries: 0,
            retry_delay_ms: 150,
            timeout_ms: 1000,
        }
    }

    fn sample_start_args() -> AxWatchStartArgs {
        AxWatchStartArgs {
            session_id: "axs-unit".to_string(),
            watch_id: Some("axw-unit".to_string()),
            events: vec![
                "AXFocusedUIElementChanged".to_string(),
                "AXTitleChanged".to_string(),
            ],
            max_buffer: 64,
        }
    }

    fn sample_poll_args() -> AxWatchPollArgs {
        AxWatchPollArgs {
            watch_id: "axw-unit".to_string(),
            limit: 10,
            drain: true,
        }
    }

    fn sample_stop_args() -> AxWatchStopArgs {
        AxWatchStopArgs {
            watch_id: "axw-unit".to_string(),
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
    fn run_poll_covers_event_text_and_tsv_rejection() {
        let lock = GlobalStateLock::new();
        let _mode = EnvGuard::set(&lock, "CODEX_MACOS_AGENT_TEST_MODE", "1");
        let _backend = EnvGuard::set(&lock, "CODEX_MACOS_AGENT_AX_BACKEND", "hammerspoon");
        let _poll_override = EnvGuard::set(
            &lock,
            "CODEX_MACOS_AGENT_AX_WATCH_POLL_JSON",
            r#"{"watch_id":"axw-unit","events":[{"watch_id":"axw-unit","event":"AXTitleChanged","at_ms":1700000002222,"role":"AXButton","title":"Save","identifier":"save-btn","pid":2001}],"dropped":0,"running":true}"#,
        );
        let runner = RealProcessRunner;

        run_poll(
            OutputFormat::Text,
            &sample_poll_args(),
            policy(false),
            &runner,
        )
        .expect("poll text should succeed");

        let err = run_poll(
            OutputFormat::Tsv,
            &sample_poll_args(),
            policy(false),
            &runner,
        )
        .expect_err("poll tsv should be rejected");
        assert!(err.to_string().contains("windows list"));
    }
}
