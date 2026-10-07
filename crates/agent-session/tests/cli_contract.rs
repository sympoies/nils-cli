use std::process::Command;

use nils_test_support::bin;
use pretty_assertions::assert_eq;

#[test]
fn readiness_parse_errors_use_the_command_envelope() {
    for args in [
        vec!["readiness", "--format", "json", "--bogus"],
        vec![
            "--state-dir",
            "fixture-state",
            "readiness",
            "--format",
            "json",
            "--bogus",
        ],
        vec![
            "--state-dir=fixture-state",
            "--host",
            "fixture-host",
            "readiness",
            "--format",
            "json",
            "--bogus",
        ],
    ] {
        let mut command = Command::new(bin::resolve("agent-session"));
        nils_test_support::cmd::strip_ambient_managed_session_env(&mut command);
        let output = command.args(args).output().expect("readiness parse error");
        assert_eq!(output.status.code(), Some(64));
        let envelope: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(envelope["schema_version"], "cli.agent-session.readiness.v1");
        assert_eq!(envelope["ok"], false);
        assert_eq!(envelope["error"]["code"], "parse-error");
    }
    let output = Command::new(bin::resolve("agent-session"))
        .args([
            "--state-dir",
            "readiness",
            "message",
            "send",
            "--format",
            "json",
            "--bogus",
        ])
        .output()
        .unwrap();
    let envelope: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        envelope["schema_version"], "cli.agent-session.message-send.v1",
        "a global option value must not select readiness"
    );
}

#[test]
fn service_mailbox_surface_requires_no_managed_sender() {
    let mut command = Command::new(bin::resolve("agent-session"));
    nils_test_support::cmd::strip_ambient_managed_session_env(&mut command);
    let output = command
        .args(["message", "service-send", "--help"])
        .output()
        .expect("service mailbox help");
    assert!(
        output.status.success(),
        "service submission must be discoverable"
    );
    let help = String::from_utf8_lossy(&output.stdout);
    for flag in [
        "--service",
        "--to",
        "--to-machine",
        "--body-file",
        "--expires-in",
        "--idempotency-key",
    ] {
        assert!(help.contains(flag), "missing {flag}: {help}");
    }
    assert!(
        !help.contains("--from"),
        "a service must not borrow a session"
    );
    assert!(
        !help.contains("--capability-file"),
        "session capabilities are not service credentials"
    );
}

#[test]
fn remote_mailbox_surface_is_discoverable() {
    let output = Command::new(bin::resolve("agent-session"))
        .args(["message", "send", "--help"])
        .output()
        .expect("agent-session help");
    assert!(String::from_utf8_lossy(&output.stdout).contains("--to-machine"));
}

/// The `main-agent` facade ships from `nils-main-agent`; its CLI help contract
/// is tested there. The operator docs stay here with the orchestration engine,
/// so this checks that they still publish the same readiness default.
#[test]
fn main_agent_docs_publish_bounded_readiness_default_and_launch_only_opt_out() {
    for (name, docs) in [
        ("README", include_str!("../README.md")),
        (
            "orchestration runbook",
            include_str!("../docs/runbooks/main-agent-orchestration.md"),
        ),
    ] {
        assert!(
            docs.contains("defaults to waiting up to 5 minutes"),
            "{name} must publish the same omitted readiness default as CLI help"
        );
        assert!(
            docs.contains("`--await-ready 0`"),
            "{name} must publish the explicit launch-only opt-out"
        );
    }
}

#[test]
fn account_surface_is_discoverable() {
    let output = Command::new(bin::resolve("agent-session"))
        .args(["account", "--help"])
        .output()
        .expect("agent-session account help");
    let help = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{help}");
    assert!(help.contains("show"), "{help}");
    assert!(help.contains("switch"), "{help}");

    let output = Command::new(bin::resolve("agent-session"))
        .args(["account", "switch", "--help"])
        .output()
        .expect("agent-session account switch help");
    let help = String::from_utf8_lossy(&output.stdout);
    for flag in [
        "--account",
        "--expected-incarnation",
        "--tmux-bin",
        "--format",
    ] {
        assert!(help.contains(flag), "missing {flag}: {help}");
    }
}

#[test]
fn account_commands_report_typed_json_errors() {
    let state = tempfile::TempDir::new().expect("state dir");
    for (args, schema) in [
        (
            vec!["account", "show", "missing-session"],
            "cli.agent-session.account-show.v1",
        ),
        (
            vec!["account", "switch", "missing-session", "--account", "beta"],
            "cli.agent-session.account-switch.v1",
        ),
    ] {
        let output = Command::new(bin::resolve("agent-session"))
            .arg("--state-dir")
            .arg(state.path())
            .args(&args)
            .args(["--format", "json"])
            .output()
            .expect("agent-session account");
        assert!(!output.status.success(), "{args:?}");
        let body: serde_json::Value =
            serde_json::from_slice(&output.stdout).expect("json envelope");
        assert_eq!(body["schema_version"], schema, "{body}");
        assert_eq!(body["ok"], false, "{body}");
        assert_eq!(body["error"]["code"], "session-not-found", "{body}");
    }

    let output = Command::new(bin::resolve("agent-session"))
        .arg("--state-dir")
        .arg(state.path())
        .args(["account", "switch", "missing-session", "--format", "json"])
        .output()
        .expect("agent-session account switch without --account");
    assert_eq!(output.status.code(), Some(64), "--account is required");
}
