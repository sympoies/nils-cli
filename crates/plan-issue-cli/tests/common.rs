use std::path::Path;

use nils_test_support::cmd::run_resolved_in_dir;

pub struct CmdOut {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

fn run_bin(bin_name: &str, args: &[&str]) -> CmdOut {
    let cwd = Path::new(env!("CARGO_MANIFEST_DIR"));
    let output = run_resolved_in_dir(bin_name, cwd, args, &[], None);

    CmdOut {
        code: output.code,
        stdout: output.stdout_text(),
        stderr: output.stderr_text(),
    }
}

pub fn run_plan_issue(args: &[&str]) -> CmdOut {
    run_bin("plan-issue", args)
}

#[allow(dead_code)]
pub fn run_plan_issue_local(args: &[&str]) -> CmdOut {
    run_bin("plan-issue-local", args)
}
