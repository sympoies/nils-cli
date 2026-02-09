#![allow(dead_code)]

use std::path::Path;

use nils_test_support::bin::resolve;
use nils_test_support::cmd::{CmdOptions, run_with};
use nils_test_support::fs::write_text;
#[allow(unused_imports)]
pub use nils_test_support::git::git;
use nils_test_support::git::{InitRepoOptions, init_repo_with};

pub struct CmdOut {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

pub fn plan_tooling_bin() -> std::path::PathBuf {
    resolve("plan-tooling")
}

pub fn run_plan_tooling(dir: &Path, args: &[&str]) -> CmdOut {
    let options = CmdOptions::new().with_cwd(dir);
    let output = run_with(&plan_tooling_bin(), args, &options);
    CmdOut {
        code: output.code,
        stdout: output.stdout_text(),
        stderr: output.stderr_text(),
    }
}

pub fn write_file(path: &Path, contents: &str) {
    write_text(path, contents);
}

pub fn init_repo() -> tempfile::TempDir {
    init_repo_with(InitRepoOptions::new().without_branch())
}
