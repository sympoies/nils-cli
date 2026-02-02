use std::path::Path;
use std::process::Output;

use nils_test_support::bin::resolve;
use nils_test_support::cmd::{run_with, CmdOptions, CmdOutput};
pub use nils_test_support::git::git;
use nils_test_support::git::{init_repo_with, InitRepoOptions};

pub fn init_repo() -> tempfile::TempDir {
    init_repo_with(InitRepoOptions::new().with_branch("main"))
}

pub fn git_scope_bin() -> std::path::PathBuf {
    resolve("git-scope")
}

pub fn run_git_scope(dir: &Path, args: &[&str], envs: &[(&str, &str)]) -> String {
    let output = run_git_scope_output(dir, args, envs);
    String::from_utf8_lossy(&output.stdout).to_string()
}

pub fn run_git_scope_output(dir: &Path, args: &[&str], envs: &[(&str, &str)]) -> Output {
    let mut options = CmdOptions::new().with_cwd(dir);
    for (key, value) in envs {
        options = options.with_env(key, value);
    }
    let output = run_with(&git_scope_bin(), args, &options);
    if output.code != 0 {
        panic!(
            "git-scope {:?} failed: {}{}",
            args,
            output.stderr_text(),
            output.stdout_text()
        );
    }
    output_from_cmd(output)
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
