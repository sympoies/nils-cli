//! Narrow exemptions for shared build-cache servers that escaped the runtime.
use super::ProcessInventory;
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct RetainedDaemon {
    pub(super) pid: libc::pid_t,
    pub(super) command: String,
    pub(super) reason: &'static str,
}

#[cfg(target_os = "linux")]
pub(super) fn linux_shared_daemon(process: &std::path::Path) -> Option<String> {
    // Use the executable, never the mutable/truncated comm field. Keep this
    // allowlist deliberately small: a new daemon needs its own server contract.
    let executable = std::fs::read_link(process.join("exe")).ok()?;
    if executable.file_name()? != "sccache" {
        return None;
    }
    let stat = std::fs::read_to_string(process.join("stat")).ok()?;
    let (_, fields) = stat.rsplit_once(") ")?;
    if fields.split_whitespace().nth(1)? != "1" {
        return None;
    }
    // The detached server re-execs without arguments. A compiler wrapper or
    // --start-server client is still ordinary session work, even if orphaned.
    let arguments = std::fs::read(process.join("cmdline")).ok()?;
    let mut arguments = arguments.split(|byte| *byte == 0);
    if arguments.next()?.is_empty() || arguments.any(|argument| !argument.is_empty()) {
        return None;
    }
    Some("sccache".to_string())
}

pub(super) fn render_retained(inventory: &ProcessInventory) -> String {
    inventory
        .retained_shared_daemons
        .iter()
        .map(|daemon| format!("{}: {} {}\n", daemon.reason, daemon.pid, daemon.command))
        .collect()
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn missing_daemon_inspection_data_never_grants_an_exemption() {
        let temp = tempfile::tempdir().unwrap();
        let process = temp.path();
        std::os::unix::fs::symlink("/opt/bin/sccache", process.join("exe")).unwrap();
        std::fs::write(process.join("stat"), "123 (sccache) S 1 0 0").unwrap();
        std::fs::write(process.join("cmdline"), b"sccache\0").unwrap();
        for file in ["exe", "stat", "cmdline"] {
            let original = process.join(file);
            let hidden = process.join(format!("{file}.hidden"));
            std::fs::rename(&original, &hidden).unwrap();
            assert!(linux_shared_daemon(process).is_none());
            std::fs::rename(&hidden, &original).unwrap();
        }
    }
}
