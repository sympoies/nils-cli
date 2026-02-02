#![allow(dead_code)]

use std::path::Path;
use std::process::Output;

use nils_test_support::bin::resolve;
use nils_test_support::cmd::{run_with, CmdOptions, CmdOutput};
#[allow(unused_imports)]
pub use nils_test_support::git::{commit_file, git, repo_id};
use nils_test_support::git::{init_repo_with, InitRepoOptions};

pub fn init_repo() -> tempfile::TempDir {
    init_repo_with(
        InitRepoOptions::new()
            .with_branch("main")
            .with_initial_commit(),
    )
}

pub fn git_lock_bin() -> std::path::PathBuf {
    resolve("git-lock")
}

pub fn run_git_lock_output(
    dir: &Path,
    args: &[&str],
    envs: &[(&str, &str)],
    input: Option<&str>,
) -> Output {
    let mut options = CmdOptions::new().with_cwd(dir);
    for (key, value) in envs {
        options = options.with_env(key, value);
    }
    options = match input {
        Some(data) => options.with_stdin_str(data),
        None => options.with_stdin_bytes(&[]),
    };

    let output = run_with(&git_lock_bin(), args, &options);
    output_from_cmd(output)
}

pub fn run_git_lock(
    dir: &Path,
    args: &[&str],
    envs: &[(&str, &str)],
    input: Option<&str>,
) -> String {
    let output = run_git_lock_output(dir, args, envs, input);
    if !output.status.success() {
        panic!(
            "git-lock {:?} failed: {}{}",
            args,
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        );
    }
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn output_from_cmd(output: CmdOutput) -> Output {
    Output {
        status: exit_status_from_code(output.code),
        stdout: output.stdout,
        stderr: output.stderr,
    }
}

#[cfg(unix)]
fn exit_status_from_code(code: i32) -> std::process::ExitStatus {
    use std::os::unix::process::ExitStatusExt;
    let raw = if code >= 0 { code << 8 } else { 1 << 8 };
    std::process::ExitStatus::from_raw(raw)
}

#[cfg(windows)]
fn exit_status_from_code(code: i32) -> std::process::ExitStatus {
    use std::os::windows::process::ExitStatusExt;
    let raw = if code >= 0 { code as u32 } else { 1 };
    std::process::ExitStatus::from_raw(raw)
}
