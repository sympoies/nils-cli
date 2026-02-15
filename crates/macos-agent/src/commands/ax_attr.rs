use std::time::Instant;

use serde_json::Value;

use crate::backend::process::ProcessRunner;
use crate::backend::{AutoAxBackend, AxBackendAdapter};
use crate::cli::{AxAttrGetArgs, AxAttrSetArgs, AxValueType, OutputFormat};
use crate::commands::ax_common::{build_selector_from_args, build_target_from_args};
use crate::commands::{emit_json_success, reject_tsv_for_list_only};
use crate::error::CliError;
use crate::model::{
    AxAttrGetRequest, AxAttrGetResult, AxAttrSetCommandResult, AxAttrSetRequest, AxAttrSetResult,
};
use crate::retry::run_with_retry;
use crate::run::{
    ActionPolicy, action_policy_result, build_action_meta_with_attempts, next_action_id,
};

pub fn run_get(
    format: OutputFormat,
    args: &AxAttrGetArgs,
    policy: ActionPolicy,
    runner: &dyn ProcessRunner,
) -> Result<(), CliError> {
    let request = AxAttrGetRequest {
        target: build_target_from_args(&args.target)?,
        selector: build_selector_from_args(&args.selector)?,
        name: args.name.clone(),
    };

    let backend = AutoAxBackend::default();
    let result = backend.attr_get(runner, &request, policy.timeout_ms)?;
    print_get_result(format, result)
}

pub fn run_set(
    format: OutputFormat,
    args: &AxAttrSetArgs,
    policy: ActionPolicy,
    runner: &dyn ProcessRunner,
) -> Result<(), CliError> {
    let request = AxAttrSetRequest {
        target: build_target_from_args(&args.target)?,
        selector: build_selector_from_args(&args.selector)?,
        name: args.name.clone(),
        value: parse_value(args.value_type, &args.value)?,
    };

    let action_id = next_action_id("ax.attr.set");
    let started = Instant::now();
    let mut attempts_used = 0u8;
    let mut detail = AxAttrSetResult {
        node_id: request.selector.node_id.clone(),
        matched_count: 0,
        name: request.name.clone(),
        applied: false,
        value_type: value_type_name(args.value_type).to_string(),
    };

    if !policy.dry_run {
        let backend = AutoAxBackend::default();
        let retry = policy.retry_policy();
        let (backend_result, attempts) = run_with_retry(retry, || {
            backend.attr_set(runner, &request, policy.timeout_ms)
        })?;
        attempts_used = attempts;
        detail = backend_result;
    }

    let result = AxAttrSetCommandResult {
        detail,
        policy: action_policy_result(policy),
        meta: build_action_meta_with_attempts(action_id, started, policy, attempts_used),
    };

    print_set_result(format, result)
}

fn value_type_name(value_type: AxValueType) -> &'static str {
    match value_type {
        AxValueType::String => "string",
        AxValueType::Number => "number",
        AxValueType::Bool => "bool",
        AxValueType::Json => "json",
        AxValueType::Null => "null",
    }
}

fn parse_value(value_type: AxValueType, raw: &str) -> Result<Value, CliError> {
    match value_type {
        AxValueType::String => Ok(Value::String(raw.to_string())),
        AxValueType::Number => {
            if let Ok(integer) = raw.parse::<i64>() {
                return Ok(Value::Number(integer.into()));
            }
            let float = raw
                .parse::<f64>()
                .map_err(|_| CliError::usage("--value is not a valid number"))?;
            let number = serde_json::Number::from_f64(float)
                .ok_or_else(|| CliError::usage("--value is not a finite number"))?;
            Ok(Value::Number(number))
        }
        AxValueType::Bool => {
            let normalized = raw.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "true" => Ok(Value::Bool(true)),
                "false" => Ok(Value::Bool(false)),
                _ => Err(CliError::usage(
                    "--value must be true or false for --value-type bool",
                )),
            }
        }
        AxValueType::Json => serde_json::from_str(raw)
            .map_err(|err| CliError::usage(format!("--value is not valid json: {err}"))),
        AxValueType::Null => Ok(Value::Null),
    }
}

fn print_get_result(format: OutputFormat, result: AxAttrGetResult) -> Result<(), CliError> {
    match format {
        OutputFormat::Json => {
            emit_json_success("ax.attr.get", result)?;
        }
        OutputFormat::Text => {
            println!(
                "ax.attr.get\tnode_id={}\tname={}\tmatched_count={}\tvalue={}",
                result.node_id.unwrap_or_default(),
                result.name,
                result.matched_count,
                result.value
            );
        }
        OutputFormat::Tsv => {
            return reject_tsv_for_list_only();
        }
    }

    Ok(())
}

fn print_set_result(format: OutputFormat, result: AxAttrSetCommandResult) -> Result<(), CliError> {
    match format {
        OutputFormat::Json => {
            emit_json_success("ax.attr.set", result)?;
        }
        OutputFormat::Text => {
            println!(
                "ax.attr.set\tnode_id={}\tname={}\tmatched_count={}\tapplied={}\tvalue_type={}\taction_id={}\telapsed_ms={}",
                result.detail.node_id.clone().unwrap_or_default(),
                result.detail.name,
                result.detail.matched_count,
                result.detail.applied,
                result.detail.value_type,
                result.meta.action_id,
                result.meta.elapsed_ms
            );
        }
        OutputFormat::Tsv => {
            return reject_tsv_for_list_only();
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use nils_test_support::{EnvGuard, GlobalStateLock};
    use serde_json::json;

    use super::{parse_value, run_get, run_set, value_type_name};
    use crate::backend::process::RealProcessRunner;
    use crate::cli::{AxAttrGetArgs, AxAttrSetArgs, AxValueType, OutputFormat};
    use crate::run::ActionPolicy;

    fn policy(dry_run: bool) -> ActionPolicy {
        ActionPolicy {
            dry_run,
            retries: 0,
            retry_delay_ms: 150,
            timeout_ms: 1000,
        }
    }

    fn sample_get_args() -> AxAttrGetArgs {
        AxAttrGetArgs {
            selector: crate::cli::AxSelectorArgs {
                node_id: Some("1.1".to_string()),
                ..crate::cli::AxSelectorArgs::default()
            },
            target: crate::cli::AxTargetArgs::default(),
            name: "AXRole".to_string(),
        }
    }

    fn sample_set_args(value_type: AxValueType, value: &str) -> AxAttrSetArgs {
        AxAttrSetArgs {
            selector: crate::cli::AxSelectorArgs {
                node_id: Some("1.1".to_string()),
                ..crate::cli::AxSelectorArgs::default()
            },
            target: crate::cli::AxTargetArgs::default(),
            name: "AXValue".to_string(),
            value: value.to_string(),
            value_type,
        }
    }

    #[test]
    fn parse_value_covers_supported_types() {
        assert_eq!(
            parse_value(AxValueType::String, "hello").expect("string"),
            json!("hello")
        );
        assert_eq!(
            parse_value(AxValueType::Number, "42").expect("integer"),
            json!(42)
        );
        assert_eq!(
            parse_value(AxValueType::Bool, "true").expect("bool"),
            json!(true)
        );
        assert_eq!(
            parse_value(AxValueType::Json, "{\"k\":1}").expect("json"),
            json!({"k": 1})
        );
        assert_eq!(
            parse_value(AxValueType::Null, "ignored").expect("null"),
            serde_json::Value::Null
        );
    }

    #[test]
    fn parse_value_reports_expected_usage_errors() {
        let bool_err = parse_value(AxValueType::Bool, "maybe").expect_err("invalid bool");
        assert!(bool_err.to_string().contains("true or false"));

        let number_err = parse_value(AxValueType::Number, "NaN").expect_err("invalid number");
        assert!(number_err.to_string().contains("finite number"));

        let json_err = parse_value(AxValueType::Json, "{invalid json").expect_err("invalid json");
        assert!(json_err.to_string().contains("valid json"));
    }

    #[test]
    fn value_type_name_matches_cli_values() {
        assert_eq!(value_type_name(AxValueType::String), "string");
        assert_eq!(value_type_name(AxValueType::Number), "number");
        assert_eq!(value_type_name(AxValueType::Bool), "bool");
        assert_eq!(value_type_name(AxValueType::Json), "json");
        assert_eq!(value_type_name(AxValueType::Null), "null");
    }

    #[test]
    fn run_get_and_set_return_usage_error_for_tsv_format() {
        let lock = GlobalStateLock::new();
        let _mode = EnvGuard::set(&lock, "AGENTS_MACOS_AGENT_TEST_MODE", "1");
        let runner = RealProcessRunner;

        let get_err = run_get(
            OutputFormat::Tsv,
            &sample_get_args(),
            policy(false),
            &runner,
        )
        .expect_err("tsv should be rejected");
        assert!(get_err.to_string().contains("windows list"));

        let set_err = run_set(
            OutputFormat::Tsv,
            &sample_set_args(AxValueType::String, "hello"),
            policy(true),
            &runner,
        )
        .expect_err("tsv should be rejected");
        assert!(set_err.to_string().contains("windows list"));
    }
}
