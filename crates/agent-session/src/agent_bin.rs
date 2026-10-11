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

#[cfg(test)]
mod tests {
    use super::resolve_agent_bin;
    use crate::cli::AgentKind;
    use nils_test_support::{EnvGuard, GlobalStateLock};
    use pretty_assertions::assert_eq;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::process::{Command, Stdio};

    const AGENTS: [(AgentKind, &str); 3] = [
        (AgentKind::Codex, "AGENT_SESSION_CODEX_BIN"),
        (AgentKind::Claude, "AGENT_SESSION_CLAUDE_BIN"),
        (AgentKind::Dsh, "AGENT_SESSION_DSH_BIN"),
    ];

    #[test]
    fn unpinned_agent_binaries_never_resolve_through_path() {
        let lock = GlobalStateLock::new();
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let planted = tmp.path().join("bin");
        std::fs::create_dir(&planted).expect("planted bin directory");
        for (agent, _) in AGENTS {
            let marker = tmp.path().join(format!("{}-ran", agent.as_str()));
            let fake = planted.join(agent.as_str());
            std::fs::write(&fake, format!("#!/bin/sh\n: > '{}'\n", marker.display()))
                .expect("planted provider");
            std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755))
                .expect("executable planted provider");
        }
        // Only the planted fakes and system tools are reachable, so no
        // installed provider CLI can run even if a planted fake fails.
        let path = format!("{}:/usr/bin:/bin", planted.display());
        let _path = EnvGuard::set(&lock, "PATH", &path);

        for (agent, env_key) in AGENTS {
            let _unpinned = EnvGuard::remove(&lock, env_key);
            let resolved = resolve_agent_bin(agent, None);
            // Execute it the way the version and app-server capability probes do.
            let _ = Command::new(&resolved)
                .arg("--version")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            let marker = tmp.path().join(format!("{}-ran", agent.as_str()));
            assert!(
                !marker.exists(),
                "unpinned {} resolved through PATH to {}",
                agent.as_str(),
                resolved.display()
            );
        }
    }

    #[test]
    fn explicit_and_pinned_agent_binaries_are_preserved() {
        let lock = GlobalStateLock::new();
        for (agent, env_key) in AGENTS {
            let pinned = format!("/pinned/{}", agent.as_str());
            let _pinned = EnvGuard::set(&lock, env_key, &pinned);
            assert_eq!(resolve_agent_bin(agent, None), Path::new(&pinned));
            assert_eq!(
                resolve_agent_bin(agent, Some(Path::new("/explicit/agent"))),
                Path::new("/explicit/agent")
            );
        }
    }
}
