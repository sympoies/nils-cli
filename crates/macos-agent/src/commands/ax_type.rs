use std::time::Instant;

use crate::backend::process::ProcessRunner;
use crate::backend::{AutoAxBackend, AxBackendAdapter};
use crate::cli::{AxPostconditionArgs, AxTypeArgs, OutputFormat};
use crate::commands::ax_common::{
    AxActionGateOptions, AxPostconditionCheck, AxPostconditionOptions, build_selector_from_args,
    build_target_from_args, evaluate_selector_against_backend, parse_postcondition_expected_value,
    run_action_gates, run_postconditions, selector_selection_error,
};
use crate::commands::{emit_json_success, reject_tsv_for_list_only};
use crate::error::CliError;
use crate::model::{AxSelector, AxTypeCommandResult, AxTypeRequest, AxTypeResult};
use crate::retry::run_with_retry;
use crate::run::{
    ActionPolicy, action_policy_result, build_action_meta_with_attempts, next_action_id,
};

pub fn run(
    format: OutputFormat,
    args: &AxTypeArgs,
    policy: ActionPolicy,
    runner: &dyn ProcessRunner,
) -> Result<(), CliError> {
    let mut request = build_request(args)?;
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
        selector_explain: None,
        gates: None,
        postconditions: None,
    };
    let gate_options = build_gate_options(args);
    let postcondition_options =
        build_postcondition_options(&args.postcondition, args.wait_timeout_ms, args.wait_poll_ms)?;

    if !policy.dry_run {
        let backend = AutoAxBackend::default();
        let gate_result = run_action_gates(
            "ax.type",
            runner,
            &backend,
            &request.target,
            &request.selector,
            gate_options,
            policy.timeout_ms,
        )?;
        let selector_evaluation = evaluate_selector_against_backend(
            runner,
            &backend,
            &request.target,
            &request.selector,
            policy.timeout_ms,
        )?;
        if let Some(error) =
            selector_selection_error("ax.type", selector_evaluation.selection_status)
        {
            return Err(error);
        }
        let selected_node_id = selector_evaluation
            .selected_node_id
            .clone()
            .ok_or_else(|| {
                CliError::ax_contract_failure("ax.type", "selector evaluation returned no node")
            })?;
        request.selector = AxSelector {
            node_id: Some(selected_node_id.clone()),
            ..AxSelector::default()
        };

        let retry = policy.retry_policy();
        let (backend_result, attempts) = run_with_retry(retry, || {
            backend.type_text(runner, &request, policy.timeout_ms)
        })?;
        attempts_used = attempts;
        detail = backend_result;
        if detail.node_id.is_none() {
            detail.node_id = Some(selected_node_id.clone());
        }
        if detail.matched_count == 0 {
            detail.matched_count = selector_evaluation.matched_count;
        }
        detail.selector_explain = if format == OutputFormat::Json {
            selector_evaluation.explain
        } else {
            None
        };
        detail.gates = gate_result;
        detail.postconditions = run_postconditions(
            "ax.type",
            runner,
            &backend,
            &request.target,
            &selected_node_id,
            &postcondition_options,
            policy.timeout_ms,
        )?;
    }

    let result = AxTypeCommandResult {
        detail,
        policy: action_policy_result(policy),
        meta: build_action_meta_with_attempts(action_id, started, policy, attempts_used),
    };

    match format {
        OutputFormat::Json => {
            emit_json_success("ax.type", result)?;
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
            return reject_tsv_for_list_only();
        }
    }

    Ok(())
}

fn build_request(args: &AxTypeArgs) -> Result<AxTypeRequest, CliError> {
    let target = build_target_from_args(&args.target)?;
    let selector = build_selector_from_args(&args.selector)?;
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

fn build_gate_options(args: &AxTypeArgs) -> AxActionGateOptions {
    AxActionGateOptions {
        app_active: args.gate.gate_app_active,
        window_present: args.gate.gate_window_present,
        ax_present: args.gate.gate_ax_present,
        ax_unique: args.gate.gate_ax_unique,
        timeout_ms: args.wait_timeout_ms.unwrap_or(args.gate.gate_timeout_ms),
        poll_ms: args.wait_poll_ms.unwrap_or(args.gate.gate_poll_ms),
    }
}

fn build_postcondition_options(
    args: &AxPostconditionArgs,
    wait_timeout_ms: Option<u64>,
    wait_poll_ms: Option<u64>,
) -> Result<AxPostconditionOptions, CliError> {
    let mut checks = Vec::new();
    if let Some(expected) = args.postcondition_focused {
        checks.push(AxPostconditionCheck::Focused(expected));
    }
    if let (Some(name), Some(raw_expected)) = (
        args.postcondition_attribute.as_ref(),
        args.postcondition_attribute_value.as_ref(),
    ) {
        if name.trim().is_empty() {
            return Err(CliError::usage("--postcondition-attribute cannot be empty"));
        }
        checks.push(AxPostconditionCheck::AttributeValue {
            name: name.clone(),
            expected: parse_postcondition_expected_value(raw_expected),
        });
    }
    Ok(AxPostconditionOptions {
        checks,
        timeout_ms: wait_timeout_ms.unwrap_or(args.postcondition_timeout_ms),
        poll_ms: wait_poll_ms.unwrap_or(args.postcondition_poll_ms),
    })
}
