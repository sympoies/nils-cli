//! Provider CLI binary resolution for launches and capability probes.

use std::path::{Path, PathBuf};

use crate::cli::AgentKind;

pub(crate) fn resolve_agent_bin(agent: AgentKind, explicit: Option<&Path>) -> PathBuf {
    if let Some(path) = explicit {
        return path.to_path_buf();
    }
    let env_key = match agent {
        AgentKind::Codex => "AGENT_SESSION_CODEX_BIN",
        AgentKind::Claude => "AGENT_SESSION_CLAUDE_BIN",
        // A dsh pane always launches its profile's agent_bin; the resolved
        // name is only ever used in typed-refusal diagnostics.
        AgentKind::Dsh => "AGENT_SESSION_DSH_BIN",
    };
    crate::non_empty_env(env_key)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(agent.as_str()))
}
