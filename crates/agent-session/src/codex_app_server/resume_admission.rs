//! Account capability probes shared by ordinary starts and canonical resume.
use super::*;

/// Provider capability only: no socket directory, operation or credentials.
pub(crate) fn resume_transport_supported(agent_bin: &Path) -> bool {
    app_server_probe(agent_bin).capabilities.transport
}

pub(super) fn bounded_command_output(
    agent_bin: &Path,
    args: &[&str],
) -> Option<std::process::Output> {
    bounded_command_output_with_timeout(agent_bin, args, APP_SERVER_CAPABILITY_PROBE_TIMEOUT)
}

/// Start-only readiness includes the ordinary socket-root preparation.
pub(crate) fn resume_runtime_ready(agent_bin: &Path, state_dir: &Path) -> bool {
    account_binding_readiness(agent_bin, state_dir).supported
}
