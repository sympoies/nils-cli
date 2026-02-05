use anyhow::{anyhow, Context, Result};
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

pub fn cmd_exists(cmd: &str) -> bool {
    find_in_path(cmd).is_some()
}

pub fn find_in_path(cmd: &str) -> Option<PathBuf> {
    nils_common::process::find_in_path(cmd)
}

pub fn run_output(cmd: &str, args: &[&str]) -> Result<Output> {
    Command::new(cmd)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| format!("spawn {cmd}"))
}

pub fn run_capture(cmd: &str, args: &[&str]) -> Result<String> {
    let output = run_output(cmd, args)?;
    if !output.status.success() {
        return Err(anyhow!(
            "{cmd} failed: {}{}",
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nils_test_support::GlobalStateLock;
    use std::fs;
    use std::fs::File;
    use tempfile::TempDir;

    #[test]
    fn find_in_path_with_explicit_missing_path_returns_none() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("missing");

        let found = find_in_path(path.to_string_lossy().as_ref());

        assert!(found.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn find_in_path_with_non_executable_file_returns_none() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("file");
        File::create(&path).expect("create file");

        let mut perms = fs::metadata(&path).expect("metadata").permissions();
        perms.set_mode(0o644);
        fs::set_permissions(&path, perms).expect("set permissions");

        let found = find_in_path(path.to_string_lossy().as_ref());

        assert!(found.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn find_in_path_with_executable_file_returns_path() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("exec");
        File::create(&path).expect("create file");

        let mut perms = fs::metadata(&path).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms).expect("set permissions");

        let found = find_in_path(path.to_string_lossy().as_ref());

        assert_eq!(found, Some(path));
    }

    #[test]
    fn run_capture_reports_spawn_failure() {
        let err = run_capture("definitely-not-a-command-xyz", &[]).expect_err("should fail");

        assert!(err
            .to_string()
            .contains("spawn definitely-not-a-command-xyz"));
    }

    #[cfg(unix)]
    #[test]
    fn run_capture_reports_nonzero_exit() {
        let _lock = GlobalStateLock::new();
        let err =
            run_capture("sh", &["-c", "printf 'oops' 1>&2; exit 1"]).expect_err("should fail");

        assert!(err.to_string().contains("failed:"));
        assert!(err.to_string().contains("oops"));
    }
}
