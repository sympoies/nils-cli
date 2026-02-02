use nils_test_support::bin;
use nils_test_support::cmd::{self, CmdOutput};
use pretty_assertions::assert_eq;
use std::path::PathBuf;

fn codex_cli_bin() -> PathBuf {
    bin::resolve("codex-cli")
}

fn run(args: &[&str]) -> CmdOutput {
    let bin = codex_cli_bin();
    cmd::run(&bin, args, &[], None)
}

fn stderr_string(output: &CmdOutput) -> String {
    output.stderr_text()
}

fn assert_exit_code(output: &CmdOutput, expected: i32) {
    assert_eq!(output.code, expected);
}

#[test]
fn dispatch_list_guidance() {
    let output = run(&["list"]);
    assert_exit_code(&output, 64);
    assert!(stderr_string(&output).contains("codex-cli: use `codex-cli help`"));
}

#[test]
fn dispatch_prompt_guidance() {
    let output = run(&["prompt"]);
    assert_exit_code(&output, 64);
    assert!(stderr_string(&output).contains("codex-cli agent prompt"));
}

#[test]
fn dispatch_advice_guidance() {
    let output = run(&["advice"]);
    assert_exit_code(&output, 64);
    assert!(stderr_string(&output).contains("codex-cli agent advice"));
}

#[test]
fn dispatch_knowledge_guidance() {
    let output = run(&["knowledge"]);
    assert_exit_code(&output, 64);
    assert!(stderr_string(&output).contains("codex-cli agent knowledge"));
}

#[test]
fn dispatch_commit_guidance() {
    let output = run(&["commit"]);
    assert_exit_code(&output, 64);
    assert!(stderr_string(&output).contains("codex-cli agent commit"));
}

#[test]
fn dispatch_auto_refresh_guidance() {
    let output = run(&["auto-refresh"]);
    assert_exit_code(&output, 64);
    assert!(stderr_string(&output).contains("codex-cli auth auto-refresh"));
}

#[test]
fn dispatch_rate_limits_guidance() {
    let output = run(&["rate-limits"]);
    assert_exit_code(&output, 64);
    assert!(stderr_string(&output).contains("codex-cli diag rate-limits"));
}
