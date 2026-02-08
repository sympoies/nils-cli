use nils_test_support::bin;
use nils_test_support::cmd::{self, CmdOutput};
use pretty_assertions::assert_eq;
use std::path::PathBuf;

fn agentctl_bin() -> PathBuf {
    bin::resolve("agentctl")
}

fn run(args: &[&str]) -> CmdOutput {
    let bin = agentctl_bin();
    cmd::run(&bin, args, &[], None)
}

#[test]
fn dispatch_help_lists_provider_neutral_groups() {
    let output = run(&["--help"]);
    assert_eq!(output.code, 0);

    let stdout = output.stdout_text();
    assert!(stdout.contains("provider"), "stdout={stdout}");
    assert!(stdout.contains("diag"), "stdout={stdout}");
    assert!(stdout.contains("debug"), "stdout={stdout}");
    assert!(stdout.contains("workflow"), "stdout={stdout}");
    assert!(stdout.contains("automation"), "stdout={stdout}");
}

#[test]
fn dispatch_unknown_command_exits_64() {
    let output = run(&["not-a-real-command"]);
    assert_eq!(output.code, 64);

    let stderr = output.stderr_text();
    assert!(
        stderr.contains("unrecognized subcommand"),
        "stderr={stderr}"
    );
    assert!(stderr.contains("not-a-real-command"), "stderr={stderr}");
}
