use nils_test_support::bin;
use nils_test_support::cmd::{self, CmdOptions, CmdOutput};
use pretty_assertions::assert_eq;
use std::path::PathBuf;

fn codex_cli_bin() -> PathBuf {
    bin::resolve("codex-cli")
}

fn run(args: &[&str]) -> CmdOutput {
    let bin = codex_cli_bin();
    cmd::run(&bin, args, &[], None)
}

fn stdout(output: &CmdOutput) -> String {
    output.stdout_text()
}

fn stderr(output: &CmdOutput) -> String {
    output.stderr_text()
}

fn assert_exit(output: &CmdOutput, code: i32) {
    assert_eq!(output.code, code);
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
fn main_agent_prompt_is_gated_and_config_show_exits_zero() {
    let options = CmdOptions::default().with_env("CODEX_ALLOW_DANGEROUS_ENABLED", "false");
    let bin = codex_cli_bin();
    let output = cmd::run_with(&bin, &["agent", "prompt", "hello"], &options);
    assert_exit(&output, 1);
    assert!(stderr(&output).contains("disabled (set CODEX_ALLOW_DANGEROUS_ENABLED=true)"));

    let output = run(&["config", "show"]);
    assert_exit(&output, 0);
}

#[test]
fn main_unknown_command_exits_64() {
    let output = run(&["not-a-real-command"]);
    assert_exit(&output, 64);
    assert!(!stderr(&output).trim().is_empty());
}
