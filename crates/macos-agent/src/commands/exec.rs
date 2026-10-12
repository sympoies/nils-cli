use std::path::Path;
use std::time::Duration;

use nils_common::fs::{SECRET_FILE_MODE, write_atomic};

use super::observation::{self, ObservationScope};
use crate::cli::ExecArgs;
use crate::commands::{hardened_env, peekaboo_binary, prepare_runtime};
use crate::error::{CliError, ErrorClass};
use crate::journal::{
    ArtifactInput, Journal, StepInput, StepStatus, sanitize_json, sanitize_output,
    sanitize_result_json,
};
use crate::model::{CoordinateClickDiagnostic, ExecutionResult, UpstreamResult};
use crate::policy;
use crate::process;
use crate::test_mode;

pub struct ExecOutcome {
    pub result: ExecutionResult,
    pub exit_code: u8,
}

pub fn run_local(
    args: &ExecArgs,
    parent_id: Option<String>,
    transport: &'static str,
) -> Result<ExecOutcome, CliError> {
    if let Err(error) = validate(args) {
        let mut journal = Journal::open(
            &args.out_dir,
            args.runtime,
            transport,
            args.evidence_mode,
            None,
        )?;
        let _ = journal.record_step(StepInput {
            parent_id,
            intent: args.intent.clone(),
            expected: args.expected.clone(),
            argv: args.argv.clone(),
            status: StepStatus::PolicyBlocked,
            failure_class: Some("policy".into()),
            duration_ms: 0,
            retries: 0,
            precondition_refs: Vec::new(),
            postcondition_refs: Vec::new(),
            snapshot_lineage: None,
            artifact_refs: Vec::new(),
        });
        let _ = journal.close();
        return Err(error);
    }
    let binary = peekaboo_binary()?;
    let mut journal = Journal::open_for_backend(
        &args.out_dir,
        args.runtime,
        transport,
        args.evidence_mode,
        None,
        &binary,
    )?;
    let runtime = match prepare_runtime(args.runtime, &binary) {
        Ok(runtime) => runtime,
        Err(error) => {
            let _ = journal.record_step(StepInput {
                parent_id,
                intent: args.intent.clone(),
                expected: args.expected.clone(),
                argv: args.argv.clone(),
                status: StepStatus::Failed,
                failure_class: Some("backend_drift".into()),
                duration_ms: 0,
                retries: 0,
                precondition_refs: Vec::new(),
                postcondition_refs: Vec::new(),
                snapshot_lineage: None,
                artifact_refs: Vec::new(),
            });
            let _ = journal.close();
            return Err(error);
        }
    };
    let (envs, removed_envs) = hardened_env(None);
    let upstream_argv = runtime.argv(&args.argv);
    let output = process::run(
        binary.path(),
        &upstream_argv,
        &envs,
        &removed_envs,
        None,
        Duration::from_secs(args.timeout_seconds.clamp(1, 3600)),
    )
    .map_err(|_| {
        CliError::upstream("failed to start the locked Peekaboo executable").with_operation("exec")
    })?;
    let mutating = policy::mutating_invocation(&args.argv);
    let unknown_mutation = mutating && (output.timed_out || output.signal.is_some());
    let mut status = if unknown_mutation {
        StepStatus::Unknown
    } else if output.timed_out || output.exit_code != 0 {
        StepStatus::Failed
    } else {
        StepStatus::Passed
    };
    let mut failure_class = if unknown_mutation {
        Some("unknown_mutation".into())
    } else if output.timed_out {
        Some("upstream_timeout".into())
    } else if output.signal.is_some() {
        Some("upstream_signal".into())
    } else if output.exit_code != 0 {
        let diagnostic = String::from_utf8_lossy(&output.stderr).to_ascii_lowercase();
        if diagnostic.contains("permission")
            || diagnostic.contains("accessibility")
            || diagnostic.contains("screen recording")
        {
            Some("permission".into())
        } else {
            Some("upstream".into())
        }
    } else {
        None
    };

    let stdout_text = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr_text = String::from_utf8_lossy(&output.stderr).to_string();
    let mut parsed = serde_json::from_str::<serde_json::Value>(stdout_text.trim()).ok();
    let observation_scope = policy::exec_command(&args.argv)
        .filter(|command| observation::is_observation(command))
        .map(|_| {
            ObservationScope::new(
                args.argv
                    .iter()
                    .any(|arg| arg == "--roi" || arg.starts_with("--roi=")),
            )
        });
    if observation_scope.is_some() {
        if let Some(value) = parsed.as_mut() {
            observation::qualify_ax_lists(value);
        }
    }
    let malformed_json = args
        .argv
        .iter()
        .any(|argument| matches!(argument.as_str(), "--json" | "--json-output"))
        && !output.timed_out
        && output.exit_code == 0
        && parsed.is_none();
    if malformed_json {
        status = StepStatus::Failed;
        failure_class = Some("upstream_malformed_json".into());
    }
    // Peekaboo v4 publishes a structured outcome envelope beside its data.
    // Classify from it when present: a refusal is not a generic upstream
    // failure, and an upstream that reports its own failure at exit zero is a
    // false success rather than a pass.
    if let Some(outcome) = parsed.as_ref().and_then(upstream_outcome) {
        // An envelope proving nothing dispatched resolves the conservative
        // unknown-mutation classification into an ordinary failure.
        let resolved_unknown = unknown_mutation && outcome.mutation_dispatched == Some(false);
        if outcome.refused {
            status = StepStatus::Failed;
            failure_class = Some("upstream_refused".into());
        } else if mutating && outcome.indeterminate && outcome.mutation_dispatched == Some(true) {
            status = StepStatus::Unknown;
            failure_class = Some("unknown_mutation".into());
        } else if resolved_unknown {
            status = StepStatus::Failed;
            failure_class = Some("upstream".into());
        } else if outcome.success == Some(false) && status == StepStatus::Passed {
            status = StepStatus::Failed;
            failure_class = Some("false_success".into());
        }
    }
    let coordinate_invocation = policy::mutating_invocation(&args.argv)
        && policy::exec_command(&args.argv).as_deref() == Some("click")
        && args
            .argv
            .iter()
            .any(|arg| arg == "--at" || arg.starts_with("--at="));
    // A failed coordinate operation without affirmative zero-dispatch evidence
    // cannot become a safe replay candidate, including after focus preparation.
    if coordinate_invocation
        && status != StepStatus::Passed
        && (output.timed_out
            || output.signal.is_some()
            || output.stdout_truncated
            || parsed
                .as_ref()
                .and_then(upstream_outcome)
                .and_then(|outcome| outcome.mutation_dispatched)
                != Some(false))
    {
        status = StepStatus::Unknown;
        failure_class = Some("unknown_mutation".into());
    }
    let debug_artifact = if args.evidence_mode == crate::cli::EvidenceMode::Debug {
        Some(write_debug_artifact(
            &args.out_dir,
            parsed.as_ref(),
            &stdout_text,
            &stderr_text,
            args.evidence_mode,
        )?)
    } else {
        None
    };
    let postconditions = if mutating {
        vec!["caller-observation-required".into()]
    } else {
        Vec::new()
    };
    let snapshot_lineage = mutating
        .then(|| policy::snapshot_lineage(&args.argv))
        .flatten();
    let preconditions = snapshot_lineage.iter().cloned().collect();
    let step = journal.record_step(StepInput {
        parent_id,
        intent: args.intent.clone(),
        expected: args.expected.clone(),
        argv: args.argv.clone(),
        status,
        failure_class,
        duration_ms: output.elapsed_ms,
        retries: 0,
        precondition_refs: preconditions,
        postcondition_refs: postconditions,
        snapshot_lineage,
        artifact_refs: debug_artifact.iter().cloned().collect(),
    })?;
    if let Some(relative) = debug_artifact {
        journal.register_artifact(ArtifactInput {
            step: &step.id,
            path: &args.out_dir.join(&relative),
            kind: "upstream_result",
            mime: "application/json",
            sensitivity: "private",
            redaction: "sanitized",
            retention: "debug",
        })?;
    }
    journal.close()?;

    let json = parsed
        .as_ref()
        .map(|value| sanitize_result_json(value, args.evidence_mode));
    let text = (json.is_none() && !stdout_text.trim().is_empty()).then(|| {
        let text = sanitize_output(stdout_text.trim(), args.evidence_mode);
        if observation_scope.is_some() {
            format!("{}\n{text}", observation::VISIBILITY_NOTICE)
        } else {
            text
        }
    });
    let diagnostic = (!stderr_text.trim().is_empty())
        .then(|| sanitize_output(stderr_text.trim(), args.evidence_mode));
    let exit_code = if output.timed_out
        || output.exit_code != 0
        || malformed_json
        || status != StepStatus::Passed
    {
        ErrorClass::Upstream.exit_code()
    } else {
        0
    };
    let coordinate_click = (status != StepStatus::Passed && coordinate_invocation).then(|| {
        coordinate_click_diagnostic(
            parsed.as_ref(),
            diagnostic.as_deref(),
            args.evidence_mode,
            output.timed_out || output.signal.is_some() || output.stdout_truncated,
        )
    });
    Ok(ExecOutcome {
        result: ExecutionResult {
            transport,
            runtime: args.runtime,
            evidence_mode: args.evidence_mode,
            journal_step: step.id,
            coordinate_click,
            observation_scope,
            upstream: UpstreamResult {
                exit_code: output.exit_code,
                signal: output.signal,
                timed_out: output.timed_out,
                stdout_truncated: output.stdout_truncated,
                stderr_truncated: output.stderr_truncated,
                json,
                text,
                diagnostic,
            },
        },
        exit_code,
    })
}

pub(super) fn coordinate_click_diagnostic(
    value: Option<&serde_json::Value>,
    diagnostic: Option<&str>,
    mode: crate::cli::EvidenceMode,
    transport_uncertain: bool,
) -> CoordinateClickDiagnostic {
    let dispatched = (!transport_uncertain)
        .then_some(value)
        .flatten()
        .and_then(|value| value.pointer("/outcome/mutation_dispatched"))
        .and_then(serde_json::Value::as_bool);
    let reason = value
        .and_then(|value| value.pointer("/outcome/refusal_reason"))
        .and_then(serde_json::Value::as_str)
        .filter(|reason| {
            matches!(
                *reason,
                "invalid_request"
                    | "permission_denied"
                    | "target_unavailable"
                    | "transport_session_unavailable"
                    | "request_cancelled"
                    | "runtime_incompatible"
                    | "foreground_consent_required"
                    | "operation_unsupported"
            )
        })
        .or_else(|| {
            value
                .and_then(|value| value.pointer("/error/code"))
                .and_then(serde_json::Value::as_str)
                .filter(|code| {
                    !code.is_empty()
                        && code.len() <= 64
                        && code.bytes().all(|byte| {
                            byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_'
                        })
                })
        })
        .unwrap_or("upstream_reason_unavailable");
    let message = value
        .and_then(|value| value.pointer("/error/message"))
        .and_then(serde_json::Value::as_str)
        .or(diagnostic)
        .map(|message| sanitize_output(message, mode));
    CoordinateClickDiagnostic {
        dispatch: match dispatched {
            Some(false) => "not_dispatched",
            Some(true) => "dispatched",
            None => "unknown",
        },
        reason: reason.to_owned(),
        message,
        guidance: if dispatched == Some(false) {
            "Input was not dispatched; focus preparation may already have changed state. Inspect the upstream cause/hint, obtain a fresh exact-window observation, and verify capture eligibility, focus, overlap and the coordinate frame (--at is window-relative unless --global). Prefer a target-bound AX action when coordinates cannot be verified."
        } else {
            "Do not replay this coordinate click. Delivery or its effect is uncertain; obtain a fresh bounded observation and verify the postcondition before any further action. Inspect the original upstream outcome and receipt; focus preparation is not proof of input delivery."
        },
    }
}

/// The parts of the Peekaboo v4 result envelope the adapter classifies on.
/// Every field is optional, so an upstream result without an envelope keeps the
/// exit-code rules unchanged.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct UpstreamOutcome {
    pub(super) success: Option<bool>,
    pub(super) refused: bool,
    pub(super) mutation_dispatched: Option<bool>,
    pub(super) indeterminate: bool,
}

pub(super) fn upstream_outcome(value: &serde_json::Value) -> Option<UpstreamOutcome> {
    let object = value.as_object()?;
    let outcome = object.get("outcome").and_then(|value| value.as_object());
    let refusal_reason = outcome
        .and_then(|outcome| outcome.get("refusal_reason"))
        .is_some_and(|reason| !reason.is_null());
    let effect = outcome
        .and_then(|outcome| outcome.get("effect"))
        .and_then(|effect| effect.as_str());
    let success = object.get("success").and_then(|value| value.as_bool());
    Some(UpstreamOutcome {
        success,
        refused: refusal_reason || effect == Some("refused"),
        mutation_dispatched: outcome
            .and_then(|outcome| outcome.get("mutation_dispatched"))
            .and_then(|value| value.as_bool()),
        // Unverifiable failures report uncertainty. Accepted background
        // deliveries retain their existing success semantics.
        indeterminate: outcome
            .and_then(|outcome| outcome.get("state"))
            .and_then(|state| state.as_str())
            == Some("indeterminate")
            || (success == Some(false) && effect == Some("unverifiable")),
    })
}

fn validate(args: &ExecArgs) -> Result<(), CliError> {
    policy::validate_exec_argv(&args.argv)?;
    if policy::mutating_invocation(&args.argv)
        && args
            .expected
            .as_deref()
            .is_none_or(|value| value.trim().is_empty())
    {
        return Err(CliError::policy(
            "mutating Peekaboo commands require --expected with an observable postcondition",
        )
        .with_operation("exec"));
    }
    Ok(())
}

fn write_debug_artifact(
    root: &Path,
    parsed: Option<&serde_json::Value>,
    stdout: &str,
    stderr: &str,
    mode: crate::cli::EvidenceMode,
) -> Result<String, CliError> {
    let filename = format!(
        "artifacts/upstream-{}-{}.json",
        test_mode::timestamp()
            .chars()
            .filter(|character| character.is_ascii_alphanumeric())
            .take(28)
            .collect::<String>(),
        std::process::id()
    );
    let payload = serde_json::json!({
        "schema_version": "macos-agent.debug-upstream.v1",
        "stdout": parsed
            .map(|value| sanitize_json(value, mode))
            .unwrap_or_else(|| serde_json::Value::String(sanitize_output(stdout, mode))),
        "stderr": sanitize_output(stderr, mode),
    });
    let body = serde_json::to_vec_pretty(&payload)
        .map_err(|_| CliError::journal("failed to encode debug upstream artifact"))?;
    write_atomic(&root.join(&filename), &body, SECRET_FILE_MODE)
        .map_err(|_| CliError::journal("failed to write debug upstream artifact"))?;
    Ok(filename)
}

#[cfg(test)]
mod tests {
    use super::{UpstreamOutcome, upstream_outcome};

    #[test]
    fn the_v4_outcome_envelope_is_read_instead_of_guessed_from_the_exit_code() {
        let refused = serde_json::json!({
            "success": false,
            "outcome": {
                "effect": "refused",
                "refusal_reason": "target_unavailable",
                "mutation_dispatched": false
            }
        });
        assert_eq!(
            upstream_outcome(&refused),
            Some(UpstreamOutcome {
                success: Some(false),
                refused: true,
                mutation_dispatched: Some(false),
                indeterminate: false,
            })
        );

        // A dispatched but unverifiable delivery is the ordinary background
        // accessibility-action result; it must never read as a refusal.
        let unverifiable = serde_json::json!({
            "success": true,
            "outcome": {"effect": "unverifiable", "mutation_dispatched": true}
        });
        assert_eq!(
            upstream_outcome(&unverifiable),
            Some(UpstreamOutcome {
                success: Some(true),
                refused: false,
                mutation_dispatched: Some(true),
                indeterminate: false,
            })
        );

        // An upstream result without an envelope leaves classification to the
        // existing exit-code rules.
        assert_eq!(
            upstream_outcome(&serde_json::json!({"data": {"ok": true}})),
            Some(UpstreamOutcome::default())
        );
        assert_eq!(upstream_outcome(&serde_json::json!([])), None);
    }
}
