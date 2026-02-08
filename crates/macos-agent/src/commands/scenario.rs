use std::process::Command;
use std::time::Instant;

use serde::Deserialize;
use serde_json::Value;

use crate::cli::{OutputFormat, ScenarioRunArgs};
use crate::commands::{emit_json_success, reject_tsv_for_list_only};
use crate::error::CliError;
use crate::model::{ScenarioRunResult, ScenarioStepResult};

#[derive(Debug, Deserialize)]
struct ScenarioFile {
    steps: Vec<ScenarioStepSpec>,
}

#[derive(Debug, Deserialize)]
struct ScenarioStepSpec {
    #[serde(default)]
    id: Option<String>,
    args: Vec<String>,
}

pub fn run(format: OutputFormat, args: &ScenarioRunArgs) -> Result<(), CliError> {
    let raw = std::fs::read_to_string(&args.file).map_err(|err| {
        CliError::runtime(format!(
            "failed to read scenario file `{}`: {err}",
            args.file.display()
        ))
        .with_operation("scenario.run")
    })?;
    let scenario: ScenarioFile = serde_json::from_str(&raw).map_err(|err| {
        CliError::usage(format!(
            "scenario file `{}` is not valid json: {err}",
            args.file.display()
        ))
        .with_operation("scenario.run")
    })?;

    if scenario.steps.is_empty() {
        return Err(
            CliError::usage("scenario file must contain at least one step")
                .with_operation("scenario.run"),
        );
    }

    let exe = std::env::current_exe().map_err(|err| {
        CliError::runtime(format!(
            "failed to resolve current executable for scenario run: {err}"
        ))
        .with_operation("scenario.run")
    })?;

    let mut step_results = Vec::with_capacity(scenario.steps.len());
    let mut first_failed_step_id = None;

    for (idx, step) in scenario.steps.iter().enumerate() {
        if step.args.is_empty() {
            return Err(
                CliError::usage(format!("scenario step {} has empty args", idx + 1))
                    .with_operation("scenario.run"),
            );
        }
        if step.args.iter().any(|arg| arg == "scenario") {
            return Err(CliError::usage(format!(
                "scenario step {} recursively invokes `scenario`; this is not allowed",
                idx + 1
            ))
            .with_operation("scenario.run")
            .with_hint("Call primitive commands directly from step args."));
        }

        let step_id = step
            .id
            .clone()
            .unwrap_or_else(|| format!("step-{}", idx + 1));

        let started = Instant::now();
        let output = Command::new(&exe)
            .args(&step.args)
            .output()
            .map_err(|err| {
                CliError::runtime(format!(
                    "failed to execute scenario step `{step_id}`: {err}"
                ))
                .with_operation("scenario.run")
            })?;

        let exit_code = output.status.code().unwrap_or(-1);
        let ok = output.status.success();
        let stdout_text = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let stderr_text = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let (operation, ax_path, fallback_used) = extract_step_telemetry(&stdout_text);
        let step_result = ScenarioStepResult {
            step_id: step_id.clone(),
            ok,
            exit_code,
            elapsed_ms: started.elapsed().as_millis() as u64,
            operation,
            ax_path,
            fallback_used,
            stdout: stdout_text,
            stderr: stderr_text,
        };

        step_results.push(step_result);
        if !ok {
            first_failed_step_id = Some(step_id);
            break;
        }
    }

    let failed_steps = step_results.iter().filter(|step| !step.ok).count();
    let passed_steps = step_results.iter().filter(|step| step.ok).count();
    let result = ScenarioRunResult {
        file: args.file.display().to_string(),
        total_steps: scenario.steps.len(),
        passed_steps,
        failed_steps,
        first_failed_step_id: first_failed_step_id.clone(),
        steps: step_results,
    };

    if failed_steps > 0 {
        let failed = first_failed_step_id.unwrap_or_else(|| "<unknown>".to_string());
        return Err(CliError::runtime(format!(
            "scenario run failed at `{failed}`"
        ))
        .with_operation("scenario.run")
        .with_hint("Inspect step stderr in scenario output and rerun with --trace for richer diagnostics."));
    }

    match format {
        OutputFormat::Json => {
            emit_json_success("scenario.run", result)?;
        }
        OutputFormat::Text => {
            println!(
                "scenario.run\tfile={}\ttotal_steps={}\tpassed_steps={}\tfailed_steps={}",
                result.file, result.total_steps, result.passed_steps, result.failed_steps
            );
        }
        OutputFormat::Tsv => {
            return reject_tsv_for_list_only();
        }
    }

    Ok(())
}

fn extract_step_telemetry(stdout: &str) -> (Option<String>, Option<String>, Option<bool>) {
    let payload: Value = match serde_json::from_str(stdout) {
        Ok(value) => value,
        Err(_) => return (None, None, None),
    };
    let command = payload
        .get("command")
        .and_then(Value::as_str)
        .map(str::to_string);
    let operation = command.clone();

    match command.as_deref() {
        Some("ax.click") => {
            let fallback = payload
                .pointer("/result/used_coordinate_fallback")
                .and_then(Value::as_bool);
            let ax_path = fallback.map(|value| {
                if value {
                    "coordinate-fallback".to_string()
                } else {
                    "ax-native".to_string()
                }
            });
            (operation, ax_path, fallback)
        }
        Some("ax.type") => {
            let fallback = payload
                .pointer("/result/used_keyboard_fallback")
                .and_then(Value::as_bool);
            let ax_path = fallback.map(|value| {
                if value {
                    "keyboard-fallback".to_string()
                } else {
                    "ax-native".to_string()
                }
            });
            (operation, ax_path, fallback)
        }
        _ => (operation, None, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_scenario_file() {
        let raw = r#"{"steps":[{"id":"s1","args":["preflight","--format","json"]}]}"#;
        let parsed: ScenarioFile = serde_json::from_str(raw).expect("scenario json should parse");
        assert_eq!(parsed.steps.len(), 1);
        assert_eq!(parsed.steps[0].id.as_deref(), Some("s1"));
    }

    #[test]
    fn extract_step_telemetry_maps_ax_fallback_modes() {
        let click_payload = r#"{"command":"ax.click","result":{"used_coordinate_fallback":true}}"#;
        let (operation, ax_path, fallback) = extract_step_telemetry(click_payload);
        assert_eq!(operation.as_deref(), Some("ax.click"));
        assert_eq!(ax_path.as_deref(), Some("coordinate-fallback"));
        assert_eq!(fallback, Some(true));

        let type_payload = r#"{"command":"ax.type","result":{"used_keyboard_fallback":false}}"#;
        let (operation, ax_path, fallback) = extract_step_telemetry(type_payload);
        assert_eq!(operation.as_deref(), Some("ax.type"));
        assert_eq!(ax_path.as_deref(), Some("ax-native"));
        assert_eq!(fallback, Some(false));
    }
}
