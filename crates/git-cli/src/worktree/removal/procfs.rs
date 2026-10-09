//! Linux visibility proof is scoped to the caller's user, rather than unrelated
//! lsof mount warnings. Unreadable live same-user processes remain unproven.
use super::{CliError, refused};
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, Read};
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const MAX_PROBE_BYTES: u64 = 64 * 1024 * 1024;

fn unavailable(stage: &str, error: impl std::fmt::Display) -> CliError {
    refused("removal-proof-unavailable", "current-user process visibility is unavailable")
        .with_hint("Retry from outside the target with readable procfs cwd, fd and maps for every live current-user process; do not ignore permission failures")
        .with_details(serde_json::json!({"backend": "procfs", "stage": stage, "reason": error.to_string()}))
}

fn active() -> CliError {
    refused("removal-process-active", "a live process has its cwd or an open file in the removal target")
        .with_hint("Run cleanup from outside the target, then close processes holding its cwd, open files or mappings and retry")
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
        (Some(false), Some(_)) | (Some(true), Some("Z" | "X")) => Ok(false),
        (Some(true), Some(_)) => Ok(true),
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

pub(super) fn processes_idle(target: &Path, root: &Path) -> Result<(), CliError> {
    let deadline = Instant::now() + Duration::from_secs(10);
    let uid = unsafe { libc::geteuid() };
    let mut budget = MAX_PROBE_BYTES;
    let mut incomplete = None;
    let processes = fs::read_dir(root).map_err(|error| unavailable("inventory", error))?;
    for entry in processes {
        if Instant::now() >= deadline {
            return Err(unavailable("inventory", "process proof timed out"));
        }
        let entry = entry.map_err(|error| unavailable("inventory", error))?;
        if !entry
            .file_name()
            .as_encoded_bytes()
            .iter()
            .all(u8::is_ascii_digit)
        {
            continue;
        }
        let process = entry.path();
        let result = (|| -> io::Result<bool> {
            let status = match read(&process.join("status"), &mut budget) {
                Ok(status) => status,
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
                Err(error) => return Err(error),
            };
            if !live_user(&status, uid)? {
                return Ok(false);
            }
            let cwd = fs::read_link(process.join("cwd"))?;
            if in_target(&cwd, target) {
                return Ok(true);
            }
            for fd in fs::read_dir(process.join("fd"))? {
                if Instant::now() >= deadline {
                    return Err(io::Error::other("process proof timed out"));
                }
                match fs::read_link(fd?.path()) {
                    Ok(path) if in_target(&path, target) => return Ok(true),
                    Ok(_) => {}
                    // A descriptor may close while we inspect it.
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                }
            }
            let maps = read(&process.join("maps"), &mut budget)?;
            Ok(maps
                .split(|byte| *byte == b'\n')
                .filter_map(mapping_path)
                .any(|path| in_target(&path, target)))
        })();
        match result {
            Ok(true) => return Err(active()),
            Ok(false) => {}
            Err(error) => {
                // Exit and zombie transitions release cwd/fds. All other loss
                // of same-user visibility remains a refusal, but continue so
                // a known holder gets the more specific active reason.
                match read(&process.join("status"), &mut budget) {
                    Err(gone) if gone.kind() == io::ErrorKind::NotFound => {}
                    Ok(status) if matches!(live_user(&status, uid), Ok(false)) => {}
                    _ => {
                        incomplete.get_or_insert_with(|| unavailable("live-process", error));
                    }
                }
            }
        }
    }
    match incomplete {
        Some(error) => Err(error),
        None => Ok(()),
    }
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
        assert!(processes_idle(&target, root.path()).is_ok());
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
                processes_idle(&target, root.path()).unwrap_err().code,
                "removal-process-active"
            );
        }
    }

    #[test]
    fn process_visibility_unreadable_live_current_user_is_retained() {
        let root = tempfile::tempdir().unwrap();
        let path = process(root.path(), unsafe { libc::geteuid() }, "S", root.path());
        // An unreadable live-process cwd is not equivalent to process exit.
        fs::remove_file(path.join("cwd")).unwrap();
        assert_eq!(
            processes_idle(&root.path().join("target"), root.path())
                .unwrap_err()
                .code,
            "removal-proof-unavailable"
        );
    }

    #[test]
    fn process_visibility_other_users_and_zombies_do_not_require_cwd() {
        for (uid, state) in [
            (unsafe { libc::geteuid() }.wrapping_add(1), "S"),
            (unsafe { libc::geteuid() }, "Z"),
        ] {
            let root = tempfile::tempdir().unwrap();
            let path = process(root.path(), uid, state, root.path());
            fs::remove_file(path.join("cwd")).unwrap();
            assert!(processes_idle(&root.path().join("target"), root.path()).is_ok());
        }
    }

    #[test]
    fn process_visibility_disappeared_process_is_not_a_holder() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("42")).unwrap();
        assert!(processes_idle(&root.path().join("target"), root.path()).is_ok());
    }

    #[test]
    fn process_visibility_unknown_identity_and_missing_inventory_are_retained() {
        let root = tempfile::tempdir().unwrap();
        let path = process(root.path(), unsafe { libc::geteuid() }, "S", root.path());
        fs::write(path.join("status"), "State: S\n").unwrap();
        assert_eq!(
            processes_idle(&root.path().join("target"), root.path())
                .unwrap_err()
                .code,
            "removal-proof-unavailable"
        );
        assert_eq!(
            processes_idle(root.path(), &root.path().join("missing"))
                .unwrap_err()
                .code,
            "removal-proof-unavailable"
        );
    }
}
