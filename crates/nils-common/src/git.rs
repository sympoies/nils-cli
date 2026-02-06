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
