use super::{
    ExecutionBudget, RuleOutcome, resolve_activity_helper, run_bounded, run_session_activity,
    simple, terminal_activity_failure_decision,
};
use crate::adapter::{hook_deadline, project_private_activity_hook};
use crate::error::HookError;
use crate::model::{Capability, DecisionAction, NormalizedRequest, Product};
use crate::observe::{self, Severity};
use std::process::{Command, Stdio};
use std::time::Duration;

const CODEX_CAPTURE_FAILED: &str = "codex-resume-identity-capture-failed";

pub(super) fn evaluate(
    capability: &Capability,
    reason_code: &str,
    request: &NormalizedRequest,
    raw: &[u8],
    execution_budget: &mut ExecutionBudget,
) -> Result<RuleOutcome, HookError> {
    match run_session_activity(request, raw, execution_budget) {
        Ok(()) => Ok(simple(DecisionAction::Allow, reason_code)),
        Err(error) if error.code == CODEX_CAPTURE_FAILED => {
            Ok(simple(DecisionAction::Warn, &error.code))
        }
        Err(error) => {
            match terminal_activity_failure_decision(capability, request.event.as_str()) {
                Some((action, code)) => Ok(simple(action, code)),
                None => Err(error),
            }
        }
    }
}

pub(super) fn deliver(
    request: &NormalizedRequest,
    raw: &[u8],
    runtime_id: &str,
) -> Result<(), HookError> {
    let Some(metadata) = project_private_activity_hook(request, raw, runtime_id) else {
        return Ok(());
    };
    let codex = request.product == Product::Codex;
    let result = resolve_activity_helper().and_then(|helper| {
        let mut command = Command::new(helper);
        command
            .args(["activity", "hook", "--agent", request.product.as_str()])
            .stdin(Stdio::piped())
            .stderr(Stdio::null());
        if codex {
            command
                .args(["--event", "CodexResumeIdentity"])
                .stdout(Stdio::piped());
            run_bounded(command, &metadata, Duration::from_secs(2), 128, false)
        } else {
            command.stdout(Stdio::null());
            run_bounded(command, &metadata, hook_deadline(&metadata), 0, false)
        }
    });
    // A fail-open hook can exit zero after a failed write. Require its
    // fixed acknowledgment and never serialize child output into diagnostics.
    if codex
        && !result.is_ok_and(|output| {
            output.status.success() && output.stdout == b"codex-resume-identity-accepted\n"
        })
    {
        observe::Record::new("activity", CODEX_CAPTURE_FAILED, Severity::Warn)
            .provider(request.product, &request.event)
            .recovery(observe::RECOVERY_HOOK_DOCTOR)
            .emit();
        return Err(HookError::runtime(
            CODEX_CAPTURE_FAILED,
            "the managed Codex resume identity could not be confirmed",
        ));
    }
    Ok(())
}
