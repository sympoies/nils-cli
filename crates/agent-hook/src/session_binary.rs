//! The same-release session binary that managed-session hooks call.
//!
//! The name comes from `nils_common::session_binary`. The binary must sit in the
//! directory of this executable, so a `PATH` entry can never stand in for it.

#[cfg(target_os = "linux")]
use std::path::Path;
use std::path::PathBuf;
#[cfg(target_os = "linux")]
use std::process::{Command, Stdio};

#[cfg(target_os = "linux")]
use nils_common::runtime_compat::classify_release;
use nils_common::session_binary::NAME;

use crate::error::HookError;

pub(crate) fn trusted_sibling() -> Result<PathBuf, HookError> {
    crate::dsh_policy::trusted_sibling(NAME)
}

/// Names the wrong pair when the sibling comes from another release generation.
///
/// Called only after a session call has failed, so the healthy path spawns
/// nothing. An unreadable or unparsable release never claims a mismatch.
#[cfg(target_os = "linux")]
pub(crate) fn release_mismatch() -> Option<String> {
    release_mismatch_for(&trusted_sibling().ok()?, env!("CARGO_PKG_VERSION"))
}

#[cfg(target_os = "linux")]
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
    classify_release(local, peer).crosses_generation().then(|| {
        format!("agent-hook {local} is paired with {NAME} {peer}; install both from one release")
    })
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    use pretty_assertions::assert_eq;

    use super::*;

    fn session_reporting(directory: &Path, output: &str) -> PathBuf {
        let path = directory.join(NAME);
        fs::write(&path, format!("#!/bin/sh\necho '{output}'\n")).expect("fixture");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).expect("mode");
        path
    }

    #[test]
    fn a_session_binary_from_another_generation_names_the_pair() {
        let directory = tempfile::tempdir().expect("tempdir");
        let binary = session_reporting(directory.path(), "x 1.40.0 (abc, rustc 1.99)");
        assert_eq!(
            release_mismatch_for(&binary, "1.39.8"),
            Some(format!(
                "agent-hook 1.39.8 is paired with {NAME} 1.40.0; install both from one release"
            ))
        );
    }

    #[test]
    fn patch_skew_and_unreadable_releases_stay_silent() {
        let directory = tempfile::tempdir().expect("tempdir");
        let patch = session_reporting(directory.path(), "x 1.39.9");
        assert_eq!(release_mismatch_for(&patch, "1.39.8"), None);
        let garbage = session_reporting(directory.path(), "not a release");
        assert_eq!(release_mismatch_for(&garbage, "1.39.8"), None);
        assert_eq!(
            release_mismatch_for(&directory.path().join("absent"), "1.39.8"),
            None
        );
    }
}
