use std::path::Path;
use std::process::Output;

use nils_test_support::bin::resolve;
use nils_test_support::cmd::{CmdOptions, run_with};
pub use nils_test_support::git::git;
use nils_test_support::git::{InitRepoOptions, init_repo_with};

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
    output.into_output()
}
