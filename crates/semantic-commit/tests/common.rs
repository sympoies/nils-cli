#![allow(dead_code)]

use std::path::Path;
use std::process::Output;

use nils_test_support::bin::resolve;
use nils_test_support::cmd::{run_with, CmdOptions, CmdOutput};
use nils_test_support::fs::{write_executable as write_executable_file, write_text};
#[allow(unused_imports)]
pub use nils_test_support::git::{git, git_output};
use nils_test_support::git::{init_repo_with, InitRepoOptions};

pub fn init_repo() -> tempfile::TempDir {
    init_repo_with(InitRepoOptions::new().with_branch("main"))
}

pub fn write_file(dir: &Path, name: &str, contents: &str) {
    let path = dir.join(name);
    write_text(&path, contents);
}

pub fn semantic_commit_bin() -> std::path::PathBuf {
    resolve("semantic-commit")
}

pub fn run_semantic_commit_output(
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
    let output = run_with(&semantic_commit_bin(), args, &options);
    output_from_cmd(output)
}

pub fn write_executable(dir: &Path, rel: &str, contents: &str) {
    let path = dir.join(rel);
    write_executable_file(&path, contents);
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
