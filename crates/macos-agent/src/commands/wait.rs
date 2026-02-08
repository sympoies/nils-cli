use crate::backend::applescript;
use crate::backend::process::RealProcessRunner;
use std::time::Instant;

use crate::cli::{OutputFormat, WaitAppActiveArgs, WaitSleepArgs, WaitWindowPresentArgs};
use crate::commands::{emit_json_success, reject_tsv_for_list_only};
use crate::error::CliError;
use crate::model::WaitResult;
use crate::targets::{self, TargetSelector};
use crate::wait;

pub fn run_sleep(format: OutputFormat, args: &WaitSleepArgs) -> Result<(), CliError> {
    let started = Instant::now();
    wait::sleep_ms(args.ms);
    let result = WaitResult {
        condition: "wait.sleep",
        attempts: 1,
        elapsed_ms: started.elapsed().as_millis() as u64,
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
    };
    emit_wait_result(format, "wait.window-present", result)
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
                "{}\tattempts={}\telapsed_ms={}",
                command, result.attempts, result.elapsed_ms
            );
        }
        OutputFormat::Tsv => {
            return reject_tsv_for_list_only();
        }
    }

    Ok(())
}
