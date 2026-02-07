use std::time::Instant;

use crate::backend::cliclick;
use crate::backend::process::ProcessRunner;
use crate::cli::{InputClickArgs, OutputFormat};
use crate::error::CliError;
use crate::model::{InputClickResult, SuccessEnvelope};
use crate::retry::run_with_retry;
use crate::run::{action_policy_result, build_action_meta, next_action_id, ActionPolicy};
use crate::wait;

pub fn run(
    format: OutputFormat,
    args: &InputClickArgs,
    policy: ActionPolicy,
    runner: &dyn ProcessRunner,
) -> Result<(), CliError> {
    if args.count == 0 {
        return Err(CliError::usage("--count must be at least 1"));
    }

    let action_id = next_action_id("input.click");
    let started = Instant::now();

    if !policy.dry_run {
        wait::sleep_ms(args.pre_wait_ms);
        let retry = policy.retry_policy();
        run_with_retry(retry, || {
            cliclick::click(
                runner,
                args.x,
                args.y,
                args.button,
                args.count,
                policy.timeout_ms,
            )
        })?;
        wait::sleep_ms(args.post_wait_ms);
    }

    let result = InputClickResult {
        x: args.x,
        y: args.y,
        button: cliclick::button_name(args.button),
        count: args.count,
        policy: action_policy_result(policy),
        meta: build_action_meta(action_id, started, policy),
    };

    match format {
        OutputFormat::Json => {
            let payload = SuccessEnvelope::new("input.click", result);
            println!(
                "{}",
                serde_json::to_string(&payload).map_err(|err| CliError::runtime(format!(
                    "failed to serialize json output: {err}"
                )))?
            );
        }
        OutputFormat::Text => {
            println!(
                "input.click\taction_id={}\tx={}\ty={}\tbutton={}\tcount={}\telapsed_ms={}",
                result.meta.action_id,
                result.x,
                result.y,
                result.button,
                result.count,
                result.meta.elapsed_ms,
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
