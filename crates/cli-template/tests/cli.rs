use nils_test_support::bin;
use nils_test_support::cmd::{self, CmdOutput};
use std::path::PathBuf;

fn cli_template_bin() -> PathBuf {
    bin::resolve("cli-template")
}

fn run(args: &[&str]) -> CmdOutput {
    let bin = cli_template_bin();
    cmd::run(&bin, args, &[], None)
}

#[test]
fn cli_template_runs_without_subcommand() {
    let output = run(&[]);
    assert_eq!(output.code, 0);
}

#[test]
fn cli_template_hello_defaults_to_world() {
    let output = run(&["hello"]);
    assert_eq!(output.code, 0);
    let stdout = output.stdout_text();
    assert!(stdout.contains("Hello, world!"), "stdout={stdout}");
}

#[test]
fn cli_template_hello_accepts_name() {
    let output = run(&["hello", "Nils"]);
    assert_eq!(output.code, 0);
    let stdout = output.stdout_text();
    assert!(stdout.contains("Hello, Nils!"), "stdout={stdout}");
}
