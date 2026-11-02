use nils_test_support::bin;
use pretty_assertions::assert_eq;
use std::process::Command;

#[test]
fn configured_executable_name_reaches_help_and_recovery_without_changing_schemas() {
    let temp = tempfile::tempdir().unwrap();
    let name = nils_common::session_binary::NAME;
    let binary = bin::resolve("agent-session");
    // Exercise the build-time identity before installing the binary under it.
    let mut built = Command::new(&binary);
    nils_test_support::cmd::strip_ambient_managed_session_env(&mut built);
    let built_help = built.args(["--help"]).output().unwrap();
    assert!(built_help.status.success());
    let built_help = String::from_utf8(built_help.stdout).unwrap();
    assert!(
        built_help.contains(&format!("  {name} start --agent codex")),
        "{built_help}"
    );
    let renamed = temp.path().join(name);
    std::fs::copy(binary, &renamed).unwrap();
    let run = |args: &[&str]| {
        let mut command = Command::new(&renamed);
        nils_test_support::cmd::strip_ambient_managed_session_env(&mut command);
        command.arg("--state-dir").arg(temp.path().join("state"));
        command.args(args).output().unwrap()
    };
    let output = run(&["--help"]);
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    for command in [
        "start", "list", "glance", "send", "resume", "restart", "metadata", "command", "attach",
        "delete",
    ] {
        assert!(help.contains(&format!("{name} {command}")), "{help}");
    }
    assert_eq!(
        help,
        built_help.replacen("Usage: agent-session ", &format!("Usage: {name} "), 1)
    );

    let output = run(&[
        "start",
        "--agent",
        "codex",
        "--workspace",
        "INVALID",
        "--format",
        "json",
    ]);
    assert_eq!(output.status.code(), Some(64));
    let body: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(body["schema_version"], "cli.agent-session.start.v1");
    assert_eq!(body["error"]["code"], "workspace-invalid");
    assert_eq!(
        body["error"]["details"]["recovery"]["command"],
        format!("{name} start --agent codex --workspace example-workspace")
    );
    for (args, schema) in [
        (
            vec!["--format", "json", "--bogus"],
            "cli.agent-session.error.v1",
        ),
        (
            vec!["readiness", "--format", "json", "--bogus"],
            "cli.agent-session.readiness.v1",
        ),
        (
            vec!["message", "send", "--format", "json", "--bogus"],
            "cli.agent-session.message-send.v1",
        ),
    ] {
        let output = run(&args);
        assert_eq!(output.status.code(), Some(64));
        let body: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(body["schema_version"], schema);
    }
}
