#![allow(dead_code)]

use std::path::Path;
use std::process::Output;

use nils_test_support::bin::resolve;
use nils_test_support::cmd::{options_in_dir_with_envs, run_resolved};
use nils_test_support::git::{InitRepoOptions, init_repo_with};
#[allow(unused_imports)]
pub use nils_test_support::git::{commit_file, git, repo_id};

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
    let mut options = options_in_dir_with_envs(dir, envs);
    options = match input {
        Some(data) => options.with_stdin_str(data),
        None => options.with_stdin_bytes(&[]),
    };

    let output = run_resolved("git-lock", args, &options);
    output.into_output()
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
