use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Output, Stdio};

pub fn run_output(args: &[&str]) -> io::Result<Output> {
    run_output_inner(None, args)
}

pub fn run_output_in(cwd: &Path, args: &[&str]) -> io::Result<Output> {
    run_output_inner(Some(cwd), args)
}

pub fn run_status_quiet(args: &[&str]) -> io::Result<ExitStatus> {
    run_status_quiet_inner(None, args)
}

pub fn run_status_quiet_in(cwd: &Path, args: &[&str]) -> io::Result<ExitStatus> {
    run_status_quiet_inner(Some(cwd), args)
}

pub fn run_status_inherit(args: &[&str]) -> io::Result<ExitStatus> {
    run_status_inherit_inner(None, args)
}

pub fn run_status_inherit_in(cwd: &Path, args: &[&str]) -> io::Result<ExitStatus> {
    run_status_inherit_inner(Some(cwd), args)
}

pub fn is_inside_work_tree() -> io::Result<bool> {
    Ok(run_status_quiet(&["rev-parse", "--is-inside-work-tree"])?.success())
}

pub fn is_inside_work_tree_in(cwd: &Path) -> io::Result<bool> {
    Ok(run_status_quiet_in(cwd, &["rev-parse", "--is-inside-work-tree"])?.success())
}

pub fn is_git_repo() -> io::Result<bool> {
    Ok(run_status_quiet(&["rev-parse", "--git-dir"])?.success())
}

pub fn is_git_repo_in(cwd: &Path) -> io::Result<bool> {
    Ok(run_status_quiet_in(cwd, &["rev-parse", "--git-dir"])?.success())
}

pub fn repo_root() -> io::Result<Option<PathBuf>> {
    let output = run_output(&["rev-parse", "--show-toplevel"])?;
    Ok(trimmed_stdout_if_success(&output).map(PathBuf::from))
}

pub fn repo_root_in(cwd: &Path) -> io::Result<Option<PathBuf>> {
    let output = run_output_in(cwd, &["rev-parse", "--show-toplevel"])?;
    Ok(trimmed_stdout_if_success(&output).map(PathBuf::from))
}

pub fn rev_parse(args: &[&str]) -> io::Result<Option<String>> {
    let output = run_output(&rev_parse_args(args))?;
    Ok(trimmed_stdout_if_success(&output))
}

pub fn rev_parse_in(cwd: &Path, args: &[&str]) -> io::Result<Option<String>> {
    let output = run_output_in(cwd, &rev_parse_args(args))?;
    Ok(trimmed_stdout_if_success(&output))
}

fn run_output_inner(cwd: Option<&Path>, args: &[&str]) -> io::Result<Output> {
    let mut cmd = Command::new("git");
    cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());
    if let Some(cwd) = cwd {
        cmd.current_dir(cwd);
    }
    cmd.output()
}

fn run_status_quiet_inner(cwd: Option<&Path>, args: &[&str]) -> io::Result<ExitStatus> {
    let mut cmd = Command::new("git");
    cmd.args(args).stdout(Stdio::null()).stderr(Stdio::null());
    if let Some(cwd) = cwd {
        cmd.current_dir(cwd);
    }
    cmd.status()
}

fn run_status_inherit_inner(cwd: Option<&Path>, args: &[&str]) -> io::Result<ExitStatus> {
    let mut cmd = Command::new("git");
    cmd.args(args)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    if let Some(cwd) = cwd {
        cmd.current_dir(cwd);
    }
    cmd.status()
}

fn rev_parse_args<'a>(args: &'a [&'a str]) -> Vec<&'a str> {
    let mut full = Vec::with_capacity(args.len() + 1);
    full.push("rev-parse");
    full.extend_from_slice(args);
    full
}

fn trimmed_stdout_if_success(output: &Output) -> Option<String> {
    if !output.status.success() {
        return None;
    }

    let trimmed = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nils_test_support::git::{InitRepoOptions, git as run_git, init_repo_with};
    use nils_test_support::{CwdGuard, GlobalStateLock};
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    #[test]
    fn run_output_in_preserves_nonzero_status() {
        let repo = init_repo_with(InitRepoOptions::new());

        let output = run_output_in(repo.path(), &["rev-parse", "--verify", "HEAD"])
            .expect("run output in repo");

        assert!(!output.status.success());
        assert!(!output.stderr.is_empty());
    }

    #[test]
    fn run_status_quiet_in_returns_success_and_failure_statuses() {
        let repo = init_repo_with(InitRepoOptions::new());

        let ok =
            run_status_quiet_in(repo.path(), &["rev-parse", "--git-dir"]).expect("status success");
        let bad = run_status_quiet_in(repo.path(), &["rev-parse", "--verify", "HEAD"])
            .expect("status failure");

        assert!(ok.success());
        assert!(!bad.success());
    }

    #[test]
    fn is_git_repo_in_and_is_inside_work_tree_in_match_repo_context() {
        let repo = init_repo_with(InitRepoOptions::new());
        let outside = TempDir::new().expect("tempdir");

        assert!(is_git_repo_in(repo.path()).expect("is_git_repo in repo"));
        assert!(is_inside_work_tree_in(repo.path()).expect("is_inside_work_tree in repo"));
        assert!(!is_git_repo_in(outside.path()).expect("is_git_repo outside repo"));
        assert!(!is_inside_work_tree_in(outside.path()).expect("is_inside_work_tree outside repo"));
    }

    #[test]
    fn repo_root_in_returns_root_or_none() {
        let repo = init_repo_with(InitRepoOptions::new());
        let outside = TempDir::new().expect("tempdir");
        let expected_root = run_git(repo.path(), &["rev-parse", "--show-toplevel"])
            .trim()
            .to_string();

        assert_eq!(
            repo_root_in(repo.path()).expect("repo_root_in repo"),
            Some(expected_root.into())
        );
        assert_eq!(
            repo_root_in(outside.path()).expect("repo_root_in outside"),
            None
        );
    }

    #[test]
    fn rev_parse_in_returns_value_or_none() {
        let repo = init_repo_with(InitRepoOptions::new().with_initial_commit());
        let head = run_git(repo.path(), &["rev-parse", "HEAD"])
            .trim()
            .to_string();

        assert_eq!(
            rev_parse_in(repo.path(), &["HEAD"]).expect("rev_parse head"),
            Some(head)
        );
        assert_eq!(
            rev_parse_in(repo.path(), &["--verify", "refs/heads/does-not-exist"])
                .expect("rev_parse missing ref"),
            None
        );
    }

    #[test]
    fn cwd_wrappers_delegate_to_in_variants() {
        let lock = GlobalStateLock::new();
        let repo = init_repo_with(InitRepoOptions::new().with_initial_commit());
        let _cwd = CwdGuard::set(&lock, repo.path()).expect("set cwd");
        let head = run_git(repo.path(), &["rev-parse", "HEAD"])
            .trim()
            .to_string();
        let root = run_git(repo.path(), &["rev-parse", "--show-toplevel"])
            .trim()
            .to_string();

        assert!(is_git_repo().expect("is_git_repo"));
        assert!(is_inside_work_tree().expect("is_inside_work_tree"));
        assert_eq!(repo_root().expect("repo_root"), Some(root.into()));
        assert_eq!(rev_parse(&["HEAD"]).expect("rev_parse"), Some(head));
    }
}
