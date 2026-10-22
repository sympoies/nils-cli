//! Session helper lifetime diagnostics for `activity doctor`.
use std::path::Path;
use std::time::Duration;

use serde::Serialize;

use super::{AgentKind, ProviderDoctor};
use crate::{CliContext, CliError};

#[derive(Clone, Debug, Serialize)]
pub(crate) struct DoctorResult {
    /// The running diagnostic binary's version, for split-install diagnosis.
    pub(crate) binary_version: String,
    pub(crate) providers: Vec<ProviderDoctor>,
    pub(crate) missing_pinned_binaries: Vec<MissingPinnedBinary>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct MissingPinnedBinary {
    session_id: String,
    binary_path: String,
    code: &'static str,
}

pub(super) fn missing_pinned_binaries(
    context: &CliContext,
    agent: Option<AgentKind>,
) -> Result<Vec<MissingPinnedBinary>, CliError> {
    let tmux = crate::resolve_tmux_bin(None);
    let mut missing = Vec::new();
    let Ok(entries) = std::fs::read_dir(context.state_dir.join("sessions")) else {
        return Ok(missing);
    };
    for entry in entries.flatten() {
        if !entry.path().join("session.json").is_file() {
            continue;
        }
        // Use the validated, read-only loader rather than list's startup
        // reconciliation. An unreadable neighbor must not hide healthy records.
        let Ok(session) = crate::load_session_record(context, &entry.file_name().to_string_lossy())
        else {
            continue;
        };
        if agent.is_some_and(|agent| agent.as_str() != session.agent)
            || crate::session_status(context, &tmux, &session) != "running"
        {
            continue;
        }
        // Read only this key, never the entire environment (which may contain
        // credentials). The per-session value also covers older runtimes.
        let Ok(binary_path) = crate::tmux_environment_value(
            &tmux,
            &crate::exact_tmux_target(&session.tmux_session),
            "AGENT_SESSION_BIN",
            Duration::from_secs(1),
        ) else {
            continue;
        };
        let path = Path::new(&binary_path);
        // Unknown/unreadable paths are not evidence that a binary was removed.
        if path.is_absolute() && matches!(path.try_exists(), Ok(false)) {
            missing.push(MissingPinnedBinary {
                session_id: session.id,
                binary_path,
                code: "pinned-agent-session-binary-missing",
            });
        }
    }
    missing.sort_by(|left, right| left.session_id.cmp(&right.session_id));
    Ok(missing)
}

pub(crate) fn render_header(result: &DoctorResult) -> String {
    let mut text = format!("binary: {}\n", result.binary_version);
    for missing in &result.missing_pinned_binaries {
        text.push_str(&format!(
            "session {}: {} ({})\n  next: restart the session to bind the installed helper\n",
            missing.session_id, missing.code, missing.binary_path
        ));
    }
    text
}
