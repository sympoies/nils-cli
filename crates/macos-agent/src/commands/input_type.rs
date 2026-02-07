use std::time::Instant;

use crate::backend::applescript;
use crate::backend::process::ProcessRunner;
use crate::cli::{InputTypeArgs, OutputFormat};
use crate::error::CliError;
use crate::model::{InputTypeResult, SuccessEnvelope};
use crate::retry::run_with_retry;
use crate::run::{
    action_policy_result, build_action_meta_with_attempts, next_action_id, ActionPolicy,
};

pub fn run(
    format: OutputFormat,
    args: &InputTypeArgs,
    policy: ActionPolicy,
    runner: &dyn ProcessRunner,
) -> Result<(), CliError> {
    if args.text.is_empty() {
        return Err(CliError::usage("--text cannot be empty"));
    }

    let action_id = next_action_id("input.type");
    let started = Instant::now();
    let mut attempts_used = 0u8;

    if !policy.dry_run {
        let retry = policy.retry_policy();
        let (_, attempts) = run_with_retry(retry, || {
            applescript::type_text(
                runner,
                &args.text,
                args.delay_ms,
                args.enter,
                policy.timeout_ms,
            )
        })?;
        attempts_used = attempts;
    }

    let result = InputTypeResult {
        text_length: args.text.chars().count(),
        enter: args.enter,
        delay_ms: args.delay_ms,
        policy: action_policy_result(policy),
        meta: build_action_meta_with_attempts(action_id, started, policy, attempts_used),
    };

    match format {
        OutputFormat::Json => {
            let payload = SuccessEnvelope::new("input.type", result);
            println!(
                "{}",
                serde_json::to_string(&payload).map_err(|err| CliError::runtime(format!(
                    "failed to serialize json output: {err}"
                )))?
            );
        }
        OutputFormat::Text => {
            println!(
                "input.type\taction_id={}\ttext_length={}\tenter={}\telapsed_ms={}",
                result.meta.action_id, result.text_length, result.enter, result.meta.elapsed_ms,
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
