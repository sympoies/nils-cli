use std::time::Instant;

use crate::backend::process::ProcessRunner;
use crate::backend::{AutoAxBackend, AxBackendAdapter};
use crate::cli::{AxTypeArgs, OutputFormat};
use crate::commands::ax_common::{build_selector, build_target, AxSelectorInput};
use crate::error::CliError;
use crate::model::{AxTypeCommandResult, AxTypeRequest, AxTypeResult, SuccessEnvelope};
use crate::retry::run_with_retry;
use crate::run::{
    action_policy_result, build_action_meta_with_attempts, next_action_id, ActionPolicy,
};

pub fn run(
    format: OutputFormat,
    args: &AxTypeArgs,
    policy: ActionPolicy,
    runner: &dyn ProcessRunner,
) -> Result<(), CliError> {
    let request = build_request(args)?;
    let action_id = next_action_id("ax.type");
    let started = Instant::now();
    let mut attempts_used = 0u8;
    let mut detail = AxTypeResult {
        node_id: request.selector.node_id.clone(),
        matched_count: 0,
        applied_via: "dry-run".to_string(),
        text_length: request.text.chars().count(),
        submitted: request.submit,
        used_keyboard_fallback: false,
    };

    if !policy.dry_run {
        let backend = AutoAxBackend::default();
        let retry = policy.retry_policy();
        let (backend_result, attempts) = run_with_retry(retry, || {
            backend.type_text(runner, &request, policy.timeout_ms)
        })?;
        attempts_used = attempts;
        detail = backend_result;
    }

    let result = AxTypeCommandResult {
        detail,
        policy: action_policy_result(policy),
        meta: build_action_meta_with_attempts(action_id, started, policy, attempts_used),
    };

    match format {
        OutputFormat::Json => {
            let payload = SuccessEnvelope::new("ax.type", result);
            println!(
                "{}",
                serde_json::to_string(&payload).map_err(|err| CliError::runtime(format!(
                    "failed to serialize json output: {err}"
                )))?
            );
        }
        OutputFormat::Text => {
            println!(
                "ax.type\taction_id={}\tnode_id={}\tapplied_via={}\ttext_length={}\telapsed_ms={}",
                result.meta.action_id,
                result.detail.node_id.unwrap_or_default(),
                result.detail.applied_via,
                result.detail.text_length,
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

fn build_request(args: &AxTypeArgs) -> Result<AxTypeRequest, CliError> {
    let target = build_target(
        args.session_id.clone(),
        args.app.clone(),
        args.bundle_id.clone(),
        args.window_title_contains.clone(),
    )?;
    let selector = build_selector(AxSelectorInput {
        node_id: args.node_id.clone(),
        role: args.role.clone(),
        title_contains: args.title_contains.clone(),
        identifier_contains: args.identifier_contains.clone(),
        value_contains: args.value_contains.clone(),
        subrole: args.subrole.clone(),
        focused: args.focused,
        enabled: args.enabled,
        nth: args.nth,
    })?;
    Ok(AxTypeRequest {
        target,
        selector,
        text: args.text.clone(),
        clear_first: args.clear_first,
        submit: args.submit,
        paste: args.paste,
        allow_keyboard_fallback: args.allow_keyboard_fallback,
    })
}
