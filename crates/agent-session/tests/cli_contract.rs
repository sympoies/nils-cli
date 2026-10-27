use std::process::Command;

use nils_test_support::bin;
use pretty_assertions::assert_eq;

#[test]
fn delete_help_documents_the_stopped_only_contract() {
    let output = Command::new(bin::resolve("agent-session"))
        .args(["delete", "--help"])
        .output()
        .expect("agent-session delete help");
    let help = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{help}");
    for phrase in [
        "--if-stopped",
        "--if-unread-empty",
        "checked atomically",
        "unread or unknown mailbox state",
        "--expected-incarnation",
        "running, working, or unknown",
        "never stops the runtime",
    ] {
        assert!(help.contains(phrase), "missing {phrase}: {help}");
    }
}

#[test]
fn delete_help_documents_explicit_split_scope_recovery() {
    let output = Command::new(bin::resolve("agent-session"))
        .args(["delete", "--help"])
        .output()
        .expect("agent-session delete help");
    let help = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{help}");
    assert!(help.contains("--recover-split-cgroup"), "{help}");
    assert!(help.contains("per-process identity checks"), "{help}");
}

#[test]
fn delete_rejects_combining_stopped_only_and_leave_processes_modes() {
    let output = Command::new(bin::resolve("agent-session"))
        .args([
            "delete",
            "guarded-delete",
            "--if-stopped",
            "--expected-incarnation",
            "fixture-incarnation",
            "--leave-processes",
            "--format",
            "json",
        ])
        .output()
        .expect("agent-session delete with conflicting modes");
    assert_eq!(output.status.code(), Some(64));
    let body: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(body["error"]["code"], "parse-error");
}

#[test]
fn delete_rejects_split_recovery_with_other_explicit_modes() {
    for mode_args in [
        vec!["--recover-split-cgroup", "--leave-processes"],
        vec![
            "--recover-split-cgroup",
            "--if-stopped",
            "--expected-incarnation",
            "fixture-incarnation",
        ],
    ] {
        let output = Command::new(bin::resolve("agent-session"))
            .args(["delete", "guarded-delete"])
            .args(mode_args)
            .args(["--format", "json"])
            .output()
            .expect("agent-session delete with conflicting recovery modes");
        assert_eq!(output.status.code(), Some(64));
        let body: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(body["error"]["code"], "parse-error");
    }
}

#[test]
fn launch_env_rejects_unapproved_keys_before_creating_a_session() {
    for agent in ["codex", "claude"] {
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

#[cfg(unix)]
#[test]
fn non_utf8_root_option_values_keep_command_parse_envelopes() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    for (option, command_args, schema) in [
        (
            b"--state-dir=".as_slice(),
            &["message", "send"][..],
            "cli.agent-session.message-send.v1",
        ),
        (
            b"--host=".as_slice(),
            &["message", "send"][..],
            "cli.agent-session.message-send.v1",
        ),
        (
            b"--state-dir=".as_slice(),
            &["readiness"][..],
            "cli.agent-session.readiness.v1",
        ),
    ] {
        let mut root_option = option.to_vec();
        root_option.push(0xff);
        let mut command = Command::new(bin::resolve("agent-session"));
        nils_test_support::cmd::strip_ambient_managed_session_env(&mut command);
        let output = command
            .arg(OsString::from_vec(root_option))
            .args(command_args)
            .args(["--format", "json", "--bogus"])
            .output()
            .expect("non-UTF-8 root option parse error");
        assert_eq!(output.status.code(), Some(64));
        let envelope: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(envelope["schema_version"], schema);
        assert_eq!(envelope["error"]["code"], "parse-error");
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
                "start", "--agent", "codex", "--env", assignment, "--format", "json",
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

#[test]
fn start_rejects_the_retired_dsh_agent_value() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut command = Command::new(bin::resolve("agent-session"));
    nils_test_support::cmd::strip_ambient_managed_session_env(&mut command);
    let output = command
        .arg("--state-dir")
        .arg(tmp.path().join("state"))
        .args(["start", "--agent", "dsh", "--format", "json"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(64));
    let body: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(body["error"]["code"], "parse-error");
    assert!(!tmp.path().join("state/sessions").exists());
}

#[test]
fn retained_activity_hook_and_notify_accept_dsh_without_advertising_it() {
    use agent_session::cli::{ActivityCommand, AgentKind, Cli, Command as SessionCommand};
    use clap::{CommandFactory, Parser};

    let hook = Cli::try_parse_from(["agent-session", "activity", "hook", "--agent", "dsh"])
        .expect("retained DSH activity hook");
    let SessionCommand::Activity(activity) = hook.command else {
        panic!("activity command");
    };
    let ActivityCommand::Hook(args) = activity.command else {
        panic!("activity hook command");
    };
    assert_eq!(args.agent, AgentKind::Dsh);

    let notify = Cli::try_parse_from([
        "agent-session",
        "activity",
        "notify",
        "--agent",
        "dsh",
        "{}",
    ])
    .expect("retained DSH activity notification");
    let SessionCommand::Activity(activity) = notify.command else {
        panic!("activity command");
    };
    let ActivityCommand::Notify(args) = activity.command else {
        panic!("activity notify command");
    };
    assert_eq!(args.agent, AgentKind::Dsh);

    let help = Cli::command().render_help().to_string();
    assert!(!help.contains("dsh"));

    let completion = Command::new(bin::resolve("agent-session"))
        .args(["completion", "zsh"])
        .output()
        .expect("agent-session zsh completion");
    assert!(completion.status.success());
    let completion = String::from_utf8(completion.stdout).unwrap();
    for description in [
        "Provider whose hook payload is on stdin",
        "Provider whose notification payload is supplied as the final argument",
    ] {
        let agent_suggestion = completion
            .lines()
            .find(|line| line.contains(description))
            .unwrap_or_else(|| panic!("missing completion entry for {description}"));
        assert!(
            agent_suggestion.contains(":AGENT:(codex claude)"),
            "supported-provider completions should be preserved: {agent_suggestion}"
        );
        assert!(
            !agent_suggestion.contains("dsh"),
            "retired DSH must stay hidden from completion: {agent_suggestion}"
        );
    }
}

#[test]
fn guarded_send_clap_contract_accepts_independent_opt_ins() {
    use agent_session::cli::{Cli, Command as SessionCommand};
    use clap::{CommandFactory, Parser};
    Cli::command().debug_assert();
    let parsed = Cli::try_parse_from([
        "agent-session",
        "send",
        "fixture",
        "--text-stdin",
        "--key",
        "enter",
        "--expect-runtime",
        "runtime",
        "--expect-provider-session",
        "provider",
        "--require-idle",
        "--require-empty-composer",
        "--idempotency-key",
        "key",
        "--format",
        "json",
    ])
    .expect("guarded send");
    let SessionCommand::Send(args) = parsed.command else {
        panic!("send command");
    };
    assert_eq!(args.guards.expect_runtime.as_deref(), Some("runtime"));
    assert_eq!(
        args.guards.expect_provider_session.as_deref(),
        Some("provider")
    );
    assert_eq!(args.guards.idempotency_key.as_deref(), Some("key"));
    assert!(args.guards.require_idle && args.guards.require_empty_composer);
    let unguarded = Cli::try_parse_from(["agent-session", "send", "fixture", "--key", "enter"])
        .expect("unguarded send");
    let SessionCommand::Send(args) = unguarded.command else {
        panic!("send command");
    };
    assert!(!args.guards.require_idle && !args.guards.require_empty_composer);
    assert!(
        args.guards.expect_runtime.is_none()
            && args.guards.expect_provider_session.is_none()
            && args.guards.idempotency_key.is_none()
    );
}

#[test]
fn delete_zsh_completion_excludes_conflicting_modes_in_both_directions() {
    let output = Command::new(bin::resolve("agent-session"))
        .args(["completion", "zsh"])
        .output()
        .expect("agent-session zsh completion");
    assert!(output.status.success());
    let completion = String::from_utf8(output.stdout).unwrap();
    for (flag, conflict) in [
        ("--if-stopped", "--leave-processes"),
        ("--leave-processes", "--if-stopped"),
    ] {
        let line = completion
            .lines()
            .find(|line| line.contains(&format!("{flag}[")))
            .expect("delete flag completion");
        assert!(
            line.split(')').next().unwrap().contains(conflict),
            "missing reciprocal conflict: {line}"
        );
    }
}

#[test]
fn unread_empty_delete_requires_stopped_and_incarnation_predicates() {
    let output = Command::new(bin::resolve("agent-session"))
        .args([
            "delete",
            "guarded-delete",
            "--if-unread-empty",
            "--format",
            "json",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(64));
    let body: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(body["error"]["code"], "parse-error");
}
