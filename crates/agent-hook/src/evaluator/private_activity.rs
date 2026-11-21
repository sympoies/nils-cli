use super::{resolve_activity_helper, run_bounded};
use crate::adapter::{hook_deadline, project_private_activity_hook};
use crate::error::HookError;
use crate::model::{NormalizedRequest, Product};
use crate::observe::{self, Severity};
use std::process::{Command, Stdio};
use std::time::Duration;

pub(super) const CODEX_CAPTURE_FAILED: &str = "codex-resume-identity-capture-failed";

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
