//! Linux collision evidence; unreadable processes produce bounded warnings.
use super::{CliError, refused};
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, Read};
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const MAX_PROBE_BYTES: u64 = 64 * 1024 * 1024;

fn active(pid: u32, command: &str, kind: &str) -> CliError {
    refused(
        "removal-process-active",
        "a readable foreign process cwd, file or mapping is inside the target",
    )
    .with_details(
        serde_json::json!({"pid":pid,"command":command,"backend":"procfs","evidence":kind}),
    )
}

fn ancestor_pids(root: &Path) -> Vec<u32> {
    let mut pids = vec![std::process::id()];
    for _ in 0..128 {
        let pid = *pids.last().unwrap();
        if pid <= 1 {
            break;
        }
        let parent = fs::read_to_string(root.join(pid.to_string()).join("status"))
            .ok()
            .and_then(|s| {
                s.lines().find_map(|l| {
                    l.strip_prefix("PPid:")
                        .and_then(|v| v.trim().parse::<u32>().ok())
                })
            });
        let Some(parent) = parent else { break };
        if parent == 0 || pids.contains(&parent) {
            break;
        }
        pids.push(parent);
    }
    pids
}

fn read(path: &Path, budget: &mut u64) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take(*budget + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > *budget {
        return Err(io::Error::other("process proof byte limit exceeded"));
    }
    *budget -= bytes.len() as u64;
    Ok(bytes)
}

fn in_target(path: &Path, target: &Path) -> bool {
    if path.starts_with(target) {
        return true;
    }
    // procfs appends this marker to unlinked open objects.
    path.as_os_str()
        .as_encoded_bytes()
        .strip_suffix(b" (deleted)")
        .is_some_and(|bytes| PathBuf::from(OsString::from_vec(bytes.to_vec())).starts_with(target))
}

fn live_user(status: &[u8], uid: u32) -> io::Result<bool> {
    let status = std::str::from_utf8(status).map_err(io::Error::other)?;
    let mut users = None;
    let mut state = None;
    for line in status.lines() {
        if let Some(value) = line.strip_prefix("Uid:") {
            let ids = value
                .split_whitespace()
                .map(str::parse::<u32>)
                .collect::<Result<Vec<_>, _>>()
                .map_err(io::Error::other)?;
            if ids.len() != 4 {
                return Err(io::Error::other("process user identity is malformed"));
            }
            users = Some(ids.contains(&uid));
        }
        if let Some(value) = line.strip_prefix("State:") {
            state = value.split_whitespace().next();
        }
    }
    match (users, state) {
        (Some(_), Some("Z" | "X")) => Ok(false),
        (Some(_), Some(_)) => Ok(true),
        _ => Err(io::Error::other(
            "process liveness or user identity is unavailable",
        )),
    }
}

fn mapping_path(line: &[u8]) -> Option<PathBuf> {
    let mut rest = line;
    // address, permissions, offset, device and inode precede the pathname.
    for _ in 0..5 {
        rest = rest.trim_ascii_start();
        let end = rest.iter().position(u8::is_ascii_whitespace)?;
        rest = &rest[end..];
    }
    let bytes = rest.trim_ascii_start();
    if !bytes.starts_with(b"/") {
        return None;
    }
    // proc maps escapes newlines; retain raw Unix bytes and spaces.
    let mut decoded = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index..].starts_with(b"\\012") {
            decoded.push(b'\n');
            index += 4;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    Some(PathBuf::from(OsString::from_vec(decoded)))
}

pub(super) fn processes_idle(
    target: &Path,
    root: &Path,
    warnings: &mut Vec<String>,
) -> Result<(), CliError> {
    processes_idle_with(target, root, warnings, |path| fs::read_link(path))
}

fn processes_idle_with(
    target: &Path,
    root: &Path,
    warnings: &mut Vec<String>,
    read_link: fn(&Path) -> io::Result<PathBuf>,
) -> Result<(), CliError> {
    let deadline = Instant::now() + Duration::from_secs(10);
    let uid = unsafe { libc::geteuid() };
    let excluded = ancestor_pids(root);
    let mut budget = MAX_PROBE_BYTES;
    let mut opaque = 0usize;
    let processes = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) => {
            warnings.push(format!("procfs inventory unavailable: {error}"));
            return Ok(());
        }
    };
    for entry in processes {
        if Instant::now() >= deadline || budget == 0 {
            warnings.push("procfs scan limit reached".into());
            break;
        }
        let Ok(entry) = entry else {
            opaque += 1;
            continue;
        };
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|s| s.parse::<u32>().ok())
        else {
            continue;
        };
        if excluded.contains(&pid) {
            continue;
        }
        let process = entry.path();
        let status = match read(&process.join("status"), &mut budget) {
            Ok(status) => status,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(_) => {
                opaque += 1;
                Vec::new()
            }
        };
        match live_user(&status, uid) {
            Ok(false) => continue,
            Err(_) => opaque += 1,
            Ok(true) => {}
        }
        let command = std::str::from_utf8(&status)
            .ok()
            .and_then(|s| {
                s.lines()
                    .find_map(|l| l.strip_prefix("Name:").map(str::trim))
            })
            .unwrap_or("unknown");
        // Read each surface independently: an opaque cwd must not hide a
        // readable fd or mapping. Unknown visibility is diagnostic, not a veto.
        let mut incomplete = false;
        match read_link(&process.join("cwd")) {
            Ok(path) if in_target(&path, target) => return Err(active(pid, command, "cwd")),
            Ok(_) => {}
            Err(_) => incomplete = true,
        }
        match fs::read_dir(process.join("fd")) {
            Ok(entries) => {
                for fd in entries {
                    if Instant::now() >= deadline {
                        incomplete = true;
                        break;
                    }
                    match fd.and_then(|entry| read_link(&entry.path())) {
                        Ok(path) if in_target(&path, target) => {
                            return Err(active(pid, command, "fd"));
                        }
                        Ok(_) => {}
                        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                        Err(_) => incomplete = true,
                    }
                }
            }
            Err(_) => incomplete = true,
        }
        match read(&process.join("maps"), &mut budget) {
            Ok(maps)
                if maps
                    .split(|b| *b == b'\n')
                    .filter_map(mapping_path)
                    .any(|path| in_target(&path, target)) =>
            {
                return Err(active(pid, command, "mapping"));
            }
            Ok(_) => {}
            Err(_) => incomplete = true,
        }
        if incomplete
            && !matches!(
                read(&process.join("status"), &mut budget).map(|s| live_user(&s, uid)),
                Ok(Ok(false))
            )
            && process.exists()
        {
            opaque += 1;
        }
    }
    if opaque > 0 {
        warnings.push(format!(
            "procfs: {opaque} unreadable process entries; visibility is incomplete"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use std::os::unix::fs::symlink;

    fn process(root: &Path, uid: u32, state: &str, cwd: &Path) -> PathBuf {
        let path = root.join("42");
        fs::create_dir_all(path.join("fd")).unwrap();
        fs::write(
            path.join("status"),
            format!("State:\t{state}\nUid:\t{uid} {uid} {uid} {uid}\n"),
        )
        .unwrap();
        fs::write(path.join("maps"), "").unwrap();
        symlink(cwd, path.join("cwd")).unwrap();
        path
    }

    #[test]
    fn process_visibility_idle_and_unrelated_processes_succeed() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("target");
        let path = process(root.path(), unsafe { libc::geteuid() }, "S", root.path());
        symlink(root.path().join("target-other/file"), path.join("fd/3")).unwrap();
        assert!(processes_idle(&target, root.path(), &mut Vec::new()).is_ok());
    }

    #[test]
    fn process_visibility_detects_cwd_open_file_and_mapping() {
        for kind in ["cwd", "fd", "maps"] {
            let root = tempfile::tempdir().unwrap();
            let target = root.path().join("target with spaces");
            let path = process(
                root.path(),
                unsafe { libc::geteuid() },
                "S",
                if kind == "cwd" { &target } else { root.path() },
            );
            match kind {
                "fd" => symlink(target.join("file (deleted)"), path.join("fd/3")).unwrap(),
                "maps" => fs::write(
                    path.join("maps"),
                    format!(
                        "1-2 r--p 00000000 00:00 1 {}\n",
                        target.join("file (deleted)").display()
                    ),
                )
                .unwrap(),
                _ => {}
            }
            assert_eq!(
                processes_idle(&target, root.path(), &mut Vec::new())
                    .unwrap_err()
                    .code,
                "removal-process-active"
            );
        }
    }

    #[test]
    fn process_visibility_unreadable_warns_and_readable_fd_still_blocks() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("target");
        let path = process(root.path(), unsafe { libc::geteuid() }, "S", root.path());
        fs::remove_file(path.join("cwd")).unwrap();
        let mut warnings = Vec::new();
        assert!(processes_idle(&target, root.path(), &mut warnings).is_ok());
        assert!(!warnings.is_empty());
        symlink(target.join("file"), path.join("fd/3")).unwrap();
        assert_eq!(
            processes_idle(&target, root.path(), &mut warnings)
                .unwrap_err()
                .code,
            "removal-process-active"
        );
    }

    #[test]
    fn process_visibility_zombies_do_not_require_cwd_and_other_users_are_scanned() {
        for (uid, state) in [
            (unsafe { libc::geteuid() }.wrapping_add(1), "S"),
            (unsafe { libc::geteuid() }, "Z"),
        ] {
            let root = tempfile::tempdir().unwrap();
            let path = process(root.path(), uid, state, root.path());
            fs::remove_file(path.join("cwd")).unwrap();
            assert!(
                processes_idle(&root.path().join("target"), root.path(), &mut Vec::new()).is_ok()
            );
        }
    }

    #[test]
    fn process_visibility_readable_other_user_holder_is_named() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("target");
        process(
            root.path(),
            unsafe { libc::geteuid() }.wrapping_add(1),
            "S",
            &target,
        );
        let error = processes_idle(&target, root.path(), &mut Vec::new()).unwrap_err();
        assert_eq!(error.code, "removal-process-active");
    }

    #[test]
    fn process_visibility_disappeared_process_is_not_a_holder() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("42")).unwrap();
        assert!(processes_idle(&root.path().join("target"), root.path(), &mut Vec::new()).is_ok());
    }

    #[test]
    fn process_visibility_eacces_is_a_warning() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("target");
        process(root.path(), unsafe { libc::geteuid() }, "S", root.path());
        let mut warnings = Vec::new();
        let result = processes_idle_with(&target, root.path(), &mut warnings, |path| {
            if path.ends_with("cwd") {
                Err(io::Error::from_raw_os_error(libc::EACCES))
            } else {
                fs::read_link(path)
            }
        });
        assert!(result.is_ok());
        assert!(!warnings.is_empty());
    }

    #[test]
    fn process_visibility_unknown_identity_and_inventory_warn() {
        let root = tempfile::tempdir().unwrap();
        let path = process(root.path(), unsafe { libc::geteuid() }, "S", root.path());
        fs::write(path.join("status"), "State: S\n").unwrap();
        let mut warnings = Vec::new();
        assert!(processes_idle(&root.path().join("target"), root.path(), &mut warnings).is_ok());
        assert!(!warnings.is_empty());
        warnings.clear();
        assert!(processes_idle(root.path(), &root.path().join("missing"), &mut warnings).is_ok());
        assert!(!warnings.is_empty());
    }
}
