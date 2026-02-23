use std::path::Path;
use std::process::Output;

use nils_test_support::bin::resolve;
use nils_test_support::cmd::{options_in_dir_with_envs, run_resolved};
pub use nils_test_support::git::git;
use nils_test_support::git::{InitRepoOptions, init_repo_with};

pub fn init_repo() -> tempfile::TempDir {
    init_repo_with(InitRepoOptions::new().with_branch("main"))
}

#[allow(dead_code)]
pub fn git_scope_bin() -> std::path::PathBuf {
    resolve("git-scope")
}

pub fn run_git_scope(dir: &Path, args: &[&str], envs: &[(&str, &str)]) -> String {
    let output = run_git_scope_output(dir, args, envs);
    String::from_utf8_lossy(&output.stdout).to_string()
}

pub fn run_git_scope_output(dir: &Path, args: &[&str], envs: &[(&str, &str)]) -> Output {
    let options = options_in_dir_with_envs(dir, envs);
    let output = run_resolved("git-scope", args, &options);
    if output.code != 0 {
        panic!(
            "git-scope {:?} failed: {}{}",
            args,
            output.stderr_text(),
            output.stdout_text()
        );
    }
    output.into_output()
}

#[allow(dead_code)]
pub fn resolve_path_command(cmd: &str) -> String {
    nils_common::process::find_in_path(cmd)
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_else(|| panic!("{cmd} not found in PATH for tests"))
}
