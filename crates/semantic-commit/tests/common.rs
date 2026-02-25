#![allow(dead_code)]

use std::path::Path;
use std::process::Output;

use nils_test_support::cmd::{options_in_dir_with_envs, run_resolved};
use nils_test_support::fs::{write_executable as write_executable_file, write_text};
use nils_test_support::git::{InitRepoOptions, init_repo_with};
#[allow(unused_imports)]
pub use nils_test_support::git::{git, git_output};

pub fn init_repo() -> tempfile::TempDir {
    init_repo_with(InitRepoOptions::new().with_branch("main"))
}

pub fn write_file(dir: &Path, name: &str, contents: &str) {
    let path = dir.join(name);
    write_text(&path, contents);
}

pub fn run_semantic_commit_output(
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
    let output = run_resolved("semantic-commit", args, &options);
    output.into_output()
}

pub fn write_executable(dir: &Path, rel: &str, contents: &str) {
    let path = dir.join(rel);
    write_executable_file(&path, contents);
}
