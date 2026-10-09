use std::process::Command;

use nils_test_support::bin;
use pretty_assertions::assert_eq;

#[test]
fn launch_env_rejects_unapproved_keys_before_creating_a_session() {
    for agent in ["codex", "claude", "dsh"] {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = tmp.path().join("state");
        let mut command = Command::new(bin::resolve("agent-session"));
        nils_test_support::cmd::strip_ambient_managed_session_env(&mut command);
        let output = command
            .env_remove("AGENT_SESSION_LAUNCH_ENV_ALLOWLIST")
            .arg("--state-dir")
            .arg(&state)
            .args([
                "start",
                "--agent",
                agent,
                "--env",
                "API_TOKEN=private-fixture-value",
                "--format",
                "json",
            ])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(64));
        let body: serde_json::Value =
            serde_json::from_slice(&output.stdout).expect("typed launch env error");
        assert_eq!(body["error"]["code"], "launch-env-key-refused");
        assert!(!String::from_utf8_lossy(&output.stdout).contains("private-fixture-value"));
        assert!(!state.join("sessions").exists());
    }
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

#[test]
fn conversation_clear_and_rebind_are_discoverable() {
    for verb in ["clear", "rebind"] {
        let output = Command::new(bin::resolve("agent-session"))
            .args([verb, "--help"])
            .output()
            .expect("conversation lifecycle help");
        assert!(output.status.success(), "missing supported {verb} command");
        let help = String::from_utf8_lossy(&output.stdout);
        for flag in ["--expect-idle", "--format", "--tmux-bin"] {
            assert!(help.contains(flag), "missing {flag}: {help}");
        }
    }
}

#[test]
fn launch_env_allowlist_is_configurable_and_refuses_credentials() {
    for (allowlist, assignment, code) in [
        (
            r#"["AGENT_RUNTIME_FEATURE"]"#,
            "AGENT_RUNTIME_FEATURE=enabled",
            "unsupported-start-agent",
        ),
        (
            r#"[]"#,
            "AGENT_RUNTIME_SUPPRESS_MEMORY=1",
            "launch-env-key-refused",
        ),
        (
            r#"["AGENT_RUNTIME_API_TOKEN"]"#,
            "AGENT_RUNTIME_API_TOKEN=private-fixture-value",
            "launch-env-allowlist-invalid",
        ),
        (
            r#"["HOME"]"#,
            "HOME=private-fixture-value",
            "launch-env-allowlist-invalid",
        ),
        (
            "invalid",
            "AGENT_RUNTIME_SUPPRESS_MEMORY=1",
            "launch-env-allowlist-invalid",
        ),
    ] {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut command = Command::new(bin::resolve("agent-session"));
        nils_test_support::cmd::strip_ambient_managed_session_env(&mut command);
        let output = command
            .arg("--state-dir")
            .arg(tmp.path().join("state"))
            .args([
                "start", "--agent", "dsh", "--env", assignment, "--format", "json",
            ])
            .env("AGENT_SESSION_LAUNCH_ENV_ALLOWLIST", allowlist)
            .output()
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(body["error"]["code"], code);
        assert!(!String::from_utf8_lossy(&output.stdout).contains("private-fixture-value"));
        assert!(!tmp.path().join("state/sessions").exists());
    }
}
