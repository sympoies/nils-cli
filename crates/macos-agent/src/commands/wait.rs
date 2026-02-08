use crate::backend::applescript;
use crate::backend::process::{ProcessRunner, RealProcessRunner};
use crate::backend::AutoAxBackend;
use std::time::Instant;

use crate::cli::{
    OutputFormat, WaitAppActiveArgs, WaitAxPresentArgs, WaitAxUniqueArgs, WaitSleepArgs,
    WaitWindowPresentArgs,
};
use crate::commands::ax_common::{
    build_selector_from_args, build_target_from_args, evaluate_selector_against_backend,
};
use crate::commands::{emit_json_success, reject_tsv_for_list_only};
use crate::error::CliError;
use crate::model::WaitResult;
use crate::run::ActionPolicy;
use crate::targets::{self, TargetSelector};
use crate::wait;

pub fn run_sleep(format: OutputFormat, args: &WaitSleepArgs) -> Result<(), CliError> {
    let started = Instant::now();
    wait::sleep_ms(args.ms);
    let result = WaitResult {
        condition: "wait.sleep",
        attempts: 1,
        elapsed_ms: started.elapsed().as_millis() as u64,
        terminal_status: "satisfied",
        matched_count: None,
        selector_explain: None,
    };
    emit_wait_result(format, "wait.sleep", result)
}

pub fn run_app_active(format: OutputFormat, args: &WaitAppActiveArgs) -> Result<(), CliError> {
    let runner = RealProcessRunner;
    let probe_timeout_ms = args.timeout_ms.max(2_000);
    let check = || {
        if let Some(app) = args.app.as_deref() {
            applescript::frontmost_app_name(&runner, probe_timeout_ms)
                .map(|frontmost| frontmost.eq_ignore_ascii_case(app))
        } else if let Some(bundle_id) = args.bundle_id.as_deref() {
            applescript::frontmost_bundle_id(&runner, probe_timeout_ms)
                .map(|frontmost| frontmost.eq_ignore_ascii_case(bundle_id))
        } else {
            Ok(false)
        }
    };

    let outcome = wait::wait_until("app-active", args.timeout_ms, args.poll_ms, check)?;
    let result = WaitResult {
        condition: "wait.app-active",
        attempts: outcome.attempts,
        elapsed_ms: outcome.elapsed_ms,
        terminal_status: "satisfied",
        matched_count: None,
        selector_explain: None,
    };
    emit_wait_result(format, "wait.app-active", result)
}

pub fn run_window_present(
    format: OutputFormat,
    args: &WaitWindowPresentArgs,
) -> Result<(), CliError> {
    let selector = TargetSelector {
        window_id: args.window_id,
        active_window: args.active_window,
        app: args.app.clone(),
        window_name: args.window_name.clone(),
    };

    let check = || targets::window_present(&selector);
    let outcome = wait::wait_until("window-present", args.timeout_ms, args.poll_ms, check)?;

    let result = WaitResult {
        condition: "wait.window-present",
        attempts: outcome.attempts,
        elapsed_ms: outcome.elapsed_ms,
        terminal_status: "satisfied",
        matched_count: None,
        selector_explain: None,
    };
    emit_wait_result(format, "wait.window-present", result)
}

pub fn run_ax_present(
    format: OutputFormat,
    args: &WaitAxPresentArgs,
    policy: ActionPolicy,
    runner: &dyn ProcessRunner,
) -> Result<(), CliError> {
    run_ax_selector_wait(
        format,
        "wait.ax-present",
        &args.selector,
        &args.target,
        args.timeout_ms,
        args.poll_ms,
        policy.timeout_ms,
        runner,
        |matched_count| matched_count >= 1,
    )
}

pub fn run_ax_unique(
    format: OutputFormat,
    args: &WaitAxUniqueArgs,
    policy: ActionPolicy,
    runner: &dyn ProcessRunner,
) -> Result<(), CliError> {
    run_ax_selector_wait(
        format,
        "wait.ax-unique",
        &args.selector,
        &args.target,
        args.timeout_ms,
        args.poll_ms,
        policy.timeout_ms,
        runner,
        |matched_count| matched_count == 1,
    )
}

#[allow(clippy::too_many_arguments)]
fn run_ax_selector_wait<F>(
    format: OutputFormat,
    command: &'static str,
    selector_args: &crate::cli::AxSelectorArgs,
    target_args: &crate::cli::AxTargetArgs,
    timeout_ms: u64,
    poll_ms: u64,
    backend_timeout_ms: u64,
    runner: &dyn ProcessRunner,
    predicate: F,
) -> Result<(), CliError>
where
    F: Fn(usize) -> bool,
{
    let selector = build_selector_from_args(selector_args)?;
    let target = build_target_from_args(target_args)?;
    let backend = AutoAxBackend::default();
    let mut last_matched_count = 0usize;
    let mut last_explain = None;

    let outcome = wait::wait_until(command, timeout_ms, poll_ms, || {
        let evaluation = evaluate_selector_against_backend(
            runner,
            &backend,
            &target,
            &selector,
            backend_timeout_ms,
        )?;
        last_matched_count = evaluation.matched_count;
        last_explain = evaluation.explain;
        Ok(predicate(evaluation.matched_count))
    })
    .map_err(|error| {
        error.with_operation(command).with_hint(format!(
            "Last selector match count before timeout: {last_matched_count}"
        ))
    })?;

    let result = WaitResult {
        condition: command,
        attempts: outcome.attempts,
        elapsed_ms: outcome.elapsed_ms,
        terminal_status: "satisfied",
        matched_count: Some(last_matched_count),
        selector_explain: if format == OutputFormat::Json {
            last_explain
        } else {
            None
        },
    };

    emit_wait_result(format, command, result)
}

fn emit_wait_result(
    format: OutputFormat,
    command: &'static str,
    result: WaitResult,
) -> Result<(), CliError> {
    match format {
        OutputFormat::Json => {
            emit_json_success(command, result)?;
        }
        OutputFormat::Text => {
            println!(
                "{}\tattempts={}\telapsed_ms={}\tterminal_status={}",
                command, result.attempts, result.elapsed_ms, result.terminal_status
            );
        }
        OutputFormat::Tsv => {
            return reject_tsv_for_list_only();
        }
    }

    Ok(())
}
