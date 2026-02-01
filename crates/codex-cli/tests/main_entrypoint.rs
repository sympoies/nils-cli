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

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

fn assert_exit(output: &Output, code: i32) {
    assert_eq!(output.status.code(), Some(code));
}

#[test]
fn main_no_args_prints_help_and_exits_zero() {
    let output = run(&[]);
    assert_exit(&output, 0);
    assert!(stdout(&output).contains("codex-cli"));
}

#[test]
fn main_help_legacy_redirect_exits_zero() {
    let output = run(&["help"]);
    assert_exit(&output, 0);
    assert!(stdout(&output).contains("codex-cli"));
}

#[test]
fn main_agent_and_config_without_subcommand_print_help() {
    let output = run(&["agent"]);
    assert_exit(&output, 0);
    assert!(stdout(&output).contains("Agent command group"));

    let output = run(&["config"]);
    assert_exit(&output, 0);
    assert!(stdout(&output).contains("Configuration command group"));
}

#[test]
fn main_agent_and_config_subcommands_exit_zero() {
    let output = run(&["agent", "prompt", "hello"]);
    assert_exit(&output, 0);

    let output = run(&["config", "show"]);
    assert_exit(&output, 0);
}

#[test]
fn main_unknown_command_exits_64() {
    let output = run(&["not-a-real-command"]);
    assert_exit(&output, 64);
    assert!(!stderr(&output).trim().is_empty());
}

