use serde_json::Value;

use crate::backend::process::ProcessRunner;
use crate::backend::{AutoAxBackend, AxBackendAdapter};
use crate::cli::{AxAttrGetArgs, AxAttrSetArgs, AxValueType, OutputFormat};
use crate::commands::ax_common::{build_selector, build_target, AxSelectorInput};
use crate::error::CliError;
use crate::model::{
    AxAttrGetRequest, AxAttrGetResult, AxAttrSetRequest, AxAttrSetResult, SuccessEnvelope,
};
use crate::run::ActionPolicy;

pub fn run_get(
    format: OutputFormat,
    args: &AxAttrGetArgs,
    policy: ActionPolicy,
    runner: &dyn ProcessRunner,
) -> Result<(), CliError> {
    let request = AxAttrGetRequest {
        target: build_target(
            args.session_id.clone(),
            args.app.clone(),
            args.bundle_id.clone(),
            args.window_title_contains.clone(),
        )?,
        selector: build_selector(AxSelectorInput {
            node_id: args.node_id.clone(),
            role: args.role.clone(),
            title_contains: args.title_contains.clone(),
            identifier_contains: args.identifier_contains.clone(),
            value_contains: args.value_contains.clone(),
            subrole: args.subrole.clone(),
            focused: args.focused,
            enabled: args.enabled,
            nth: args.nth,
        })?,
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
        target: build_target(
            args.session_id.clone(),
            args.app.clone(),
            args.bundle_id.clone(),
            args.window_title_contains.clone(),
        )?,
        selector: build_selector(AxSelectorInput {
            node_id: args.node_id.clone(),
            role: args.role.clone(),
            title_contains: args.title_contains.clone(),
            identifier_contains: args.identifier_contains.clone(),
            value_contains: args.value_contains.clone(),
            subrole: args.subrole.clone(),
            focused: args.focused,
            enabled: args.enabled,
            nth: args.nth,
        })?,
        name: args.name.clone(),
        value: parse_value(args.value_type, &args.value)?,
    };

    let result = if policy.dry_run {
        AxAttrSetResult {
            node_id: request.selector.node_id.clone(),
            matched_count: 0,
            name: request.name.clone(),
            applied: false,
            value_type: value_type_name(args.value_type).to_string(),
        }
    } else {
        let backend = AutoAxBackend::default();
        backend.attr_set(runner, &request, policy.timeout_ms)?
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
            let payload = SuccessEnvelope::new("ax.attr.get", result);
            println!(
                "{}",
                serde_json::to_string(&payload).map_err(|err| CliError::runtime(format!(
                    "failed to serialize json output: {err}"
                )))?
            );
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
            return Err(CliError::usage(
                "--format tsv is only supported for `windows list` and `apps list`",
            ));
        }
    }

    Ok(())
}

fn print_set_result(format: OutputFormat, result: AxAttrSetResult) -> Result<(), CliError> {
    match format {
        OutputFormat::Json => {
            let payload = SuccessEnvelope::new("ax.attr.set", result);
            println!(
                "{}",
                serde_json::to_string(&payload).map_err(|err| CliError::runtime(format!(
                    "failed to serialize json output: {err}"
                )))?
            );
        }
        OutputFormat::Text => {
            println!(
                "ax.attr.set\tnode_id={}\tname={}\tmatched_count={}\tapplied={}\tvalue_type={}",
                result.node_id.unwrap_or_default(),
                result.name,
                result.matched_count,
                result.applied,
                result.value_type
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
