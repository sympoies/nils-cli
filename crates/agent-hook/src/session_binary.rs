//! The same-release session binary that the managed-session lifecycle calls.
//!
//! The name comes from `nils_common::session_binary`. The binary must sit in the
//! directory of this executable, so a `PATH` entry can never stand in for it.

use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use nils_common::runtime_compat::{ReleaseSkew, classify_release};
use nils_common::session_binary::NAME;

use crate::error::HookError;

pub(crate) fn trusted_sibling() -> Result<PathBuf, HookError> {
    let current = std::env::current_exe()
        .ok()
        .and_then(|path| fs::canonicalize(path).ok())
        .ok_or_else(|| {
            HookError::runtime(
                "operation-lifecycle-helper-unavailable",
                "agent-hook executable is unavailable",
            )
        })?;
    sibling_in(&current, NAME)
}

/// A canonical, owner-controlled executable named `name` that stays in the
/// directory of `current`; an alias within that directory is accepted.
fn sibling_in(current: &Path, name: &str) -> Result<PathBuf, HookError> {
    let unavailable = || {
        HookError::runtime(
            "operation-lifecycle-helper-unavailable",
            "same-release session binary is unavailable",
        )
    };
    let candidate = current
        .parent()
        .map(|parent| parent.join(name))
        .and_then(|path| fs::canonicalize(path).ok())
        .ok_or_else(unavailable)?;
    let metadata = fs::metadata(&candidate).map_err(|_| unavailable())?;
    if candidate.parent() != current.parent()
        || !metadata.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.permissions().mode() & 0o022 != 0
        || metadata.permissions().mode() & 0o111 == 0
    {
        return Err(HookError::data(
            "operation-lifecycle-helper-untrusted",
            "same-release session binary is not an owner-controlled executable",
        ));
    }
    Ok(candidate)
}

/// Names the wrong pair when the sibling reports a different release.
///
/// Called only after a session call has failed, so the healthy path spawns
/// nothing. An unreadable or unparsable release never claims a mismatch.
pub(crate) fn release_mismatch() -> Option<String> {
    release_mismatch_for(&trusted_sibling().ok()?, env!("CARGO_PKG_VERSION"))
}

fn release_mismatch_for(binary: &Path, local: &str) -> Option<String> {
    let output = Command::new(binary)
        .arg("--version")
        .env_clear()
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|output| output.status.success())?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let peer = stdout.split_whitespace().nth(1)?;
    matches!(
        classify_release(local, peer),
        ReleaseSkew::Patch | ReleaseSkew::Minor | ReleaseSkew::Major
    )
    .then(|| {
        format!("agent-hook {local} is paired with {NAME} {peer}; install both from one release")
    })
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;

    use pretty_assertions::assert_eq;

    use super::*;

    fn executable(path: &Path, body: &str, mode: u32) -> PathBuf {
        fs::write(path, body).expect("fixture");
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("mode");
        path.to_path_buf()
    }

    fn session_reporting(directory: &Path, output: &str) -> PathBuf {
        executable(
            &directory.join(NAME),
            &format!("#!/bin/sh\necho '{output}'\n"),
            0o700,
        )
    }

    #[test]
    fn a_session_binary_from_another_release_names_the_pair() {
        let directory = tempfile::tempdir().expect("tempdir");
        for (reported, peer) in [
            ("x 1.40.0 (abc, rustc 1.99)", "1.40.0"),
            ("x 2.0.0", "2.0.0"),
            ("x 1.39.9", "1.39.9"),
        ] {
            let binary = session_reporting(directory.path(), reported);
            assert_eq!(
                release_mismatch_for(&binary, "1.39.8"),
                Some(format!(
                    "agent-hook 1.39.8 is paired with {NAME} {peer}; install both from one release"
                ))
            );
        }
    }

    #[test]
    fn the_same_unreadable_or_missing_release_stays_silent() {
        let directory = tempfile::tempdir().expect("tempdir");
        let same = session_reporting(directory.path(), "x 1.39.8 (abc, rustc 1.99)");
        assert_eq!(release_mismatch_for(&same, "1.39.8"), None);
        let garbage = session_reporting(directory.path(), "not a release");
        assert_eq!(release_mismatch_for(&garbage, "1.39.8"), None);
        assert_eq!(
            release_mismatch_for(&directory.path().join("absent"), "1.39.8"),
            None
        );
    }

    #[test]
    fn the_sibling_is_an_owner_controlled_executable_in_the_hook_directory() {
        let directory = tempfile::tempdir().expect("tempdir");
        let release = fs::canonicalize(directory.path()).expect("canonical");
        let hook = executable(&release.join("agent-hook"), "", 0o700);

        assert!(sibling_in(&hook, NAME).is_err(), "absent sibling");

        let real = executable(&release.join("session-real"), "", 0o700);
        symlink(&real, release.join(NAME)).expect("in-directory alias");
        assert_eq!(sibling_in(&hook, NAME).expect("alias"), real);
        fs::remove_file(release.join(NAME)).expect("remove alias");

        let elsewhere = tempfile::tempdir().expect("tempdir");
        let outside = executable(&elsewhere.path().join("session-real"), "", 0o700);
        symlink(&outside, release.join(NAME)).expect("escaping alias");
        let escaped = sibling_in(&hook, NAME).expect_err("escaping alias");
        assert_eq!(escaped.code, "operation-lifecycle-helper-untrusted");
        fs::remove_file(release.join(NAME)).expect("remove alias");

        for mode in [0o720, 0o600] {
            executable(&release.join(NAME), "", mode);
            let refused = sibling_in(&hook, NAME).expect_err("mode");
            assert_eq!(
                refused.code, "operation-lifecycle-helper-untrusted",
                "{mode:o}"
            );
        }
        executable(&release.join(NAME), "", 0o700);
        assert_eq!(sibling_in(&hook, NAME).expect("plain"), release.join(NAME));
    }
}
