use std::time::Instant;

use crate::backend::cliclick;
use crate::backend::process::ProcessRunner;
use crate::backend::{AppleScriptAxBackend, AxBackendAdapter};
use crate::cli::{AxClickArgs, MouseButton, OutputFormat};
use crate::error::CliError;
use crate::model::{
    AxClickCommandResult, AxClickRequest, AxClickResult, AxSelector, AxTarget, SuccessEnvelope,
};
use crate::retry::run_with_retry;
use crate::run::{
    action_policy_result, build_action_meta_with_attempts, next_action_id, ActionPolicy,
};

pub fn run(
    format: OutputFormat,
    args: &AxClickArgs,
    policy: ActionPolicy,
    runner: &dyn ProcessRunner,
) -> Result<(), CliError> {
    let request = build_request(args)?;
    let action_id = next_action_id("ax.click");
    let started = Instant::now();
    let mut attempts_used = 0u8;
    let mut detail = AxClickResult {
        node_id: request.selector.node_id.clone(),
        matched_count: 0,
        action: "dry-run".to_string(),
        used_coordinate_fallback: false,
        fallback_x: None,
        fallback_y: None,
    };

    if !policy.dry_run {
        let backend = AppleScriptAxBackend;
        let retry = policy.retry_policy();
        let (mut backend_result, attempts) =
            run_with_retry(retry, || backend.click(runner, &request, policy.timeout_ms))?;
        attempts_used = attempts;
        if backend_result.used_coordinate_fallback {
            let x = backend_result.fallback_x.ok_or_else(|| {
                CliError::ax_contract_failure(
                    "ax.click",
                    "backend requested coordinate fallback but x coordinate is missing",
                )
            })?;
            let y = backend_result.fallback_y.ok_or_else(|| {
                CliError::ax_contract_failure(
                    "ax.click",
                    "backend requested coordinate fallback but y coordinate is missing",
                )
            })?;
            cliclick::click(runner, x, y, MouseButton::Left, 1, policy.timeout_ms)?;
            backend_result.action = "coordinate-fallback".to_string();
        }
        detail = backend_result;
    }

    let result = AxClickCommandResult {
        detail,
        policy: action_policy_result(policy),
        meta: build_action_meta_with_attempts(action_id, started, policy, attempts_used),
    };

    match format {
        OutputFormat::Json => {
            let payload = SuccessEnvelope::new("ax.click", result);
            println!(
                "{}",
                serde_json::to_string(&payload).map_err(|err| CliError::runtime(format!(
                    "failed to serialize json output: {err}"
                )))?
            );
        }
        OutputFormat::Text => {
            println!(
                "ax.click\taction_id={}\tnode_id={}\taction={}\tmatched_count={}\telapsed_ms={}",
                result.meta.action_id,
                result.detail.node_id.unwrap_or_default(),
                result.detail.action,
                result.detail.matched_count,
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

fn build_request(args: &AxClickArgs) -> Result<AxClickRequest, CliError> {
    if args.nth == Some(0) {
        return Err(CliError::usage("--nth must be at least 1"));
    }
    Ok(AxClickRequest {
        target: AxTarget {
            app: args.app.clone(),
            bundle_id: args.bundle_id.clone(),
        },
        selector: AxSelector {
            node_id: args.node_id.clone(),
            role: args.role.clone(),
            title_contains: args.title_contains.clone(),
            nth: args.nth.map(|value| value as usize),
        },
        allow_coordinate_fallback: args.allow_coordinate_fallback,
    })
}
