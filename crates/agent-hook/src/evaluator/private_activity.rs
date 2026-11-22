use std::process::{Command, Stdio};

use super::{resolve_activity_helper, run_bounded};
use crate::adapter::{hook_deadline, project_session_model_hook};
use crate::model::NormalizedRequest;

pub(super) fn deliver(request: &NormalizedRequest, raw: &[u8], runtime_id: &str) {
    if let Some(metadata) = project_session_model_hook(request, raw, runtime_id)
        && let Ok(helper) = resolve_activity_helper()
    {
        let mut command = Command::new(helper);
        command
            .args(["activity", "hook", "--agent", request.product.as_str()])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        // Runs after mandatory activity, outside enforcement's child/output budget.
        let _ = run_bounded(command, &metadata, hook_deadline(&metadata), 0, false);
    }
}
