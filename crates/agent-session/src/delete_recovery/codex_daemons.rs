//! Provider-owned Codex daemons are shared by provider home, not by session.
use crate::*;
use std::io::Read;

/// Codex 0.162.1 publishes native process identities in its daemon PID files.
/// Both dedicated-daemon and standalone installer locations are emitted by
/// that external release. Neither display names nor inherited markers prove
/// provider ownership: bind the executable, command and native PID record.
pub(crate) fn classify(process: &Path) -> Option<&'static str> {
    let pid: libc::pid_t = process.file_name()?.to_str()?.parse().ok()?;
    let before = read_linux_process_identity(pid).ok()??;
    if before.zombie {
        return None;
    }
    let executable = fs::canonicalize(process.join("exe")).ok()?;
    let environment = fs::read(process.join("environ")).ok()?;
    let variable = |name: &[u8]| {
        environment
            .split(|byte| *byte == 0)
            .find_map(|entry| entry.strip_prefix(name))
            .filter(|value| !value.is_empty())
    };
    use std::os::unix::ffi::OsStrExt;
    let home = variable(b"CODEX_HOME=")
        .map(|value| PathBuf::from(std::ffi::OsStr::from_bytes(value)))
        .or_else(|| {
            variable(b"HOME=")
                .map(|value| PathBuf::from(std::ffi::OsStr::from_bytes(value)).join(".codex"))
        })?;
    if !home.is_absolute() {
        return None;
    }
    let home = fs::canonicalize(home).ok()?;
    // The deletion owner's configuration is independent from the candidate.
    // An arbitrary session child cannot select a new provider installation.
    let owner_home = env::var_os("CODEX_HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .or_else(|| nils_common::fs::home_dir().map(|home| home.join(".codex")))?;
    if !owner_home.is_absolute() || fs::canonicalize(owner_home).ok()? != home {
        return None;
    }
    let installation = ["app-server-daemon", "standalone"]
        .into_iter()
        .find(|kind| {
            fs::canonicalize(home.join("packages").join(kind).join("current/bin/codex"))
                .ok()
                .as_ref()
                == Some(&executable)
                && executable
                    .strip_prefix(home.join("packages").join(kind).join("releases"))
                    .ok()
                    .is_some_and(|relative| {
                        let components = relative.iter().collect::<Vec<_>>();
                        components.len() == 3 && components[1] == "bin" && components[2] == "codex"
                    })
        })?;
    let metadata = fs::metadata(&executable).ok()?;
    if !metadata.is_file() || metadata.uid() != unsafe { libc::geteuid() } {
        return None;
    }
    let bytes = fs::read(process.join("cmdline")).ok()?;
    let arguments = bytes
        .split(|byte| *byte == 0)
        .filter(|arg| !arg.is_empty())
        .map(std::str::from_utf8)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    let (label, updater) = command_kind(arguments.get(1..)?)?;
    let file = match (installation, updater) {
        ("standalone", false) => "app-server.pid",
        ("standalone", true) => "app-server-updater.pid",
        (_, false) => "daemon.pid",
        (_, true) => "daemon-updater.pid",
    };
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(home.join("app-server-daemon").join(file))
        .ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.nlink() != 1
        || metadata.len() > 8192
        || metadata.mode() & 0o022 != 0
    {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(8193).read_to_end(&mut bytes).ok()?;
    let record: Value = serde_json::from_slice(&bytes).ok()?;
    if record["pid"].as_i64() != Some(i64::from(pid))
        || record
            .pointer("/processIdentity/startTicks")
            .and_then(Value::as_u64)
            != Some(before.start_time)
        || record
            .pointer("/processIdentity/bootId")
            .and_then(Value::as_str)
            != Some(linux_boot_id().ok()?.as_str())
    {
        return None;
    }
    let after = read_linux_process_identity(pid).ok()??;
    (!after.zombie && after.start_time == before.start_time).then_some(label)
}

fn command_kind(arguments: &[&str]) -> Option<(&'static str, bool)> {
    if arguments == ["app-server", "daemon", "pid-update-loop"]
        || (arguments.len() == 5
            && arguments[..4]
                == [
                    "app-server",
                    "daemon",
                    "pid-update-loop",
                    "--restore-release",
                ])
    {
        return Some(("codex-daemon-updater", true));
    }
    if arguments.first() != Some(&"app-server") {
        return None;
    }
    let mut listen = false;
    let mut managed = false;
    let mut arguments = arguments[1..].iter().copied();
    while let Some(argument) = arguments.next() {
        match argument {
            "--listen" if !listen => {
                listen = arguments.next()? == "unix://";
                if !listen {
                    return None;
                }
            }
            "--managed-daemon" if !managed => managed = true,
            "--analytics-default-enabled" | "--remote-control" => {}
            "-c" => {
                let setting = arguments.next()?;
                if !setting.starts_with("features.")
                    || !(setting.ends_with("=true") || setting.ends_with("=false"))
                {
                    return None;
                }
            }
            _ => return None,
        }
    }
    (listen && managed).then_some(("codex-app-server", false))
}

fn scope_pids(path: &Path) -> Result<Vec<libc::pid_t>, SessionTerminationFailure> {
    let deadline = Instant::now() + Duration::from_secs(1);
    let mut directories = vec![path.to_path_buf()];
    let mut pids = Vec::new();
    let mut inspected = 0;
    while let Some(directory) = directories.pop() {
        inspected += 1;
        if inspected > 128 || Instant::now() >= deadline {
            return Err(SessionTerminationFailure::VerificationFailed);
        }
        let text = fs::read_to_string(directory.join("cgroup.procs"))
            .map_err(|_| SessionTerminationFailure::VerificationFailed)?;
        for pid in text.split_whitespace() {
            pids.push(
                pid.parse()
                    .map_err(|_| SessionTerminationFailure::VerificationFailed)?,
            );
            if pids.len() > 128 {
                return Err(SessionTerminationFailure::VerificationFailed);
            }
        }
        for entry in
            fs::read_dir(&directory).map_err(|_| SessionTerminationFailure::VerificationFailed)?
        {
            let entry = entry.map_err(|_| SessionTerminationFailure::VerificationFailed)?;
            if entry
                .file_type()
                .map_err(|_| SessionTerminationFailure::VerificationFailed)?
                .is_dir()
            {
                directories.push(entry.path());
            }
        }
    }
    pids.sort_unstable();
    pids.dedup();
    Ok(pids)
}

pub(crate) fn proven_pid(pid: libc::pid_t, start: Option<u64>) -> bool {
    let process = PathBuf::from(format!("/proc/{pid}"));
    start.is_none_or(|start| matches!(read_linux_process_identity(pid), Ok(Some(identity)) if !identity.zombie && identity.start_time == start))
        && classify(&process).is_some()
}

pub(crate) fn provider_only_scope(path: &Path) -> bool {
    let Ok(pids) = scope_pids(path) else {
        return false;
    };
    !pids.is_empty()
        && pids.iter().all(|pid| proven_pid(*pid, None))
        && scope_pids(path).ok().as_ref() == Some(&pids)
}

/// A shared provider daemon forbids freezing or killing the whole cgroup.
/// The caller instead kills the identity-checked tmux target and verifies the
/// remaining boundary; any non-provider survivor still fails closed.
pub(crate) fn shared_scope(
    identity: &TmuxRuntimeIdentity,
) -> Result<bool, SessionTerminationFailure> {
    let Some(group) = identity.control_group.as_ref() else {
        return Ok(false);
    };
    shared_group_at_root(group, Path::new("/sys/fs/cgroup"))
}

pub(crate) fn shared_group_at_root(
    group: &TmuxControlGroupIdentity,
    root: &Path,
) -> Result<bool, SessionTerminationFailure> {
    let path = linux_control_group_full_path_at_root(Path::new(&group.path), root)?;
    let metadata = match fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(_) => return Err(SessionTerminationFailure::VerificationFailed),
    };
    if metadata.dev() != group.device || metadata.ino() != group.inode {
        return Err(SessionTerminationFailure::RuntimeIdentityMismatch);
    }
    Ok(scope_pids(&path)?
        .into_iter()
        .any(|pid| proven_pid(pid, None)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn private_session_server_is_never_a_provider_daemon() {
        for arguments in [
            vec!["app-server", "--listen", "unix:///session.sock"],
            vec![
                "app-server",
                "--listen",
                "unix:///session.sock",
                "--managed-daemon",
            ],
            vec!["app-server", "--listen", "unix://"],
        ] {
            assert!(command_kind(&arguments).is_none());
        }
    }
}
