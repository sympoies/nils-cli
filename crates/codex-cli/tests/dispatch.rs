use pretty_assertions::assert_eq;
use std::path::PathBuf;
use std::process::{Command, Output};

fn codex_cli_bin() -> PathBuf {
    if let Ok(bin) = std::env::var("CARGO_BIN_EXE_codex-cli")
        .or_else(|_| std::env::var("CARGO_BIN_EXE_codex_cli"))
    {
        return PathBuf::from(bin);
    }

    let exe = std::env::current_exe().expect("current exe");
    let target_dir = exe.parent().and_then(|p| p.parent()).expect("target dir");
    let bin = target_dir.join("codex-cli");
    if bin.exists() {
        return bin;
    }

    panic!("codex-cli binary path: NotPresent");
}

fn run(args: &[&str]) -> Output {
    Command::new(codex_cli_bin())
        .args(args)
        .output()
        .expect("run codex-cli")
}

fn stderr_string(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

fn assert_exit_code(output: &Output, expected: i32) {
    assert_eq!(output.status.code(), Some(expected));
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
