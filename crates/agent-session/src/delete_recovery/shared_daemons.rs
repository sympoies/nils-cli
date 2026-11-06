//! Narrow exemptions for shared per-user daemons that escaped the runtime.
use super::ProcessInventory;
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct RetainedDaemon {
    pub(super) pid: libc::pid_t,
    pub(super) command: String,
    pub(super) reason: &'static str,
}

#[derive(Clone)]
pub(super) struct DaemonPaths {
    roots: Vec<std::path::PathBuf>,
    excluded: Vec<std::path::PathBuf>,
}

impl DaemonPaths {
    pub(super) fn for_session(cwd: &std::path::Path, state: &std::path::Path) -> Option<Self> {
        let roots = [
            "/bin",
            "/sbin",
            "/usr/bin",
            "/usr/sbin",
            "/usr/lib",
            "/usr/libexec",
            "/usr/local",
            "/opt/homebrew",
            "/opt/local",
            "/home/linuxbrew/.linuxbrew",
            "/nix/store",
        ]
        .iter()
        .filter_map(|root| std::fs::canonicalize(root).ok())
        .collect();
        let cwd = std::fs::canonicalize(cwd).ok()?;
        let worktree = cwd
            .ancestors()
            .find(|path| path.join(".git").exists())
            .unwrap_or(&cwd)
            .to_path_buf();
        let mut excluded = vec![
            worktree,
            std::fs::canonicalize(state).ok()?,
            std::fs::canonicalize(std::env::temp_dir()).ok()?,
        ];
        excluded.extend(
            ["/tmp", "/var/tmp"]
                .iter()
                .filter_map(|path| std::fs::canonicalize(path).ok()),
        );
        Some(Self { roots, excluded })
    }

    fn allows(&self, path: &std::path::Path) -> bool {
        self.roots.iter().any(|root| path.starts_with(root))
            && !self.excluded.iter().any(|root| path.starts_with(root))
    }

    #[cfg(test)]
    pub(super) fn fixture(root: &std::path::Path) -> Self {
        Self {
            roots: vec![std::fs::canonicalize(root).unwrap()],
            excluded: Vec::new(),
        }
    }
}

pub(super) fn gnupg_executable(
    path: &std::path::Path,
    policy: Option<&DaemonPaths>,
) -> Option<String> {
    let path = std::fs::canonicalize(path).ok()?;
    let name = path.file_name()?.to_str()?;
    (is_gnupg(name) && policy?.allows(&path)).then(|| name.to_owned())
}

#[cfg(target_os = "linux")]
pub(super) fn linux_shared_daemon(
    process: &std::path::Path,
    policy: Option<&DaemonPaths>,
) -> Option<String> {
    // Use the executable, never the mutable/truncated comm field. Keep this
    // allowlist deliberately small: a new daemon needs its own server contract.
    let executable = std::fs::read_link(process.join("exe")).ok()?;
    let name = executable.file_name()?.to_str()?;
    let stat = std::fs::read_to_string(process.join("stat")).ok()?;
    let (_, fields) = stat.rsplit_once(") ")?;
    let parent = fields.split_whitespace().nth(1)?;
    if parent != "1" {
        return None;
    }
    if is_gnupg(name) {
        return gnupg_executable(&process.join("exe"), policy);
    }
    if name != "sccache" {
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

pub(super) fn is_gnupg(name: &str) -> bool {
    matches!(name, "gpg-agent" | "dirmngr" | "keyboxd")
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
    fn renamed_copy_outside_package_location_is_not_exempt() {
        let temp = tempfile::tempdir().unwrap();
        let package = temp.path().join("package");
        std::fs::create_dir(&package).unwrap();
        let renamed = temp.path().join("dirmngr");
        std::fs::write(&renamed, b"fixture").unwrap();
        let link = package.join("dirmngr");
        std::os::unix::fs::symlink(&renamed, &link).unwrap();
        let policy = DaemonPaths::fixture(&package);
        assert!(gnupg_executable(&renamed, Some(&policy)).is_none());
        assert!(gnupg_executable(&link, Some(&policy)).is_none());
        let copy = package.join("keyboxd");
        std::fs::write(&copy, b"fixture").unwrap();
        let mut policy = policy;
        policy.excluded.push(package.clone());
        assert!(gnupg_executable(&copy, Some(&policy)).is_none());
    }

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
            assert!(linux_shared_daemon(process, None).is_none());
            std::fs::rename(&hidden, &original).unwrap();
        }
    }
}
