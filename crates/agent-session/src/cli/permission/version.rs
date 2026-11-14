//! Reviewed external provider range for permission decisions, independent of notices.
use crate::{CliError, SessionRecord, cli::AgentKind, send::claude_notice::version::Version};
use std::{path::Path, process::Command, time::Duration};

// Raising either bound requires a reviewed PermissionRequest contract audit.
const SUPPORTED_MIN: &str = "2.1.317";
const SUPPORTED_MAX: &str = "2.1.317";

pub(super) fn check(record: &SessionRecord) -> Result<(), CliError> {
    let binary = crate::resolve_agent_bin(
        AgentKind::Claude,
        record.agent_bin.as_deref().map(Path::new),
    );
    let mut command = Command::new(binary);
    command.arg("--version");
    let version =
        crate::run_output_with_timeout_and_strict_cap(command, Duration::from_secs(2), 256)
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| Version::parse_banner(&String::from_utf8_lossy(&o.stdout)))
            .ok_or_else(|| super::refusal("permission-provider-version-unavailable"))?;
    if !version.in_range(
        Version::parse(SUPPORTED_MIN).unwrap(),
        Version::parse(SUPPORTED_MAX).unwrap(),
    ) {
        return Err(super::refusal("permission-provider-version-unsupported"));
    }
    Ok(())
}
