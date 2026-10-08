use nils_test_support::cmd::{CmdOptions, run_resolved};
use pretty_assertions::assert_eq;
use serde_json::Value;

fn run(state: &std::path::Path, args: &[&str]) -> nils_test_support::cmd::CmdOutput {
    let mut command = vec![
        "--state-dir",
        state.to_str().unwrap(),
        "--host",
        "test-host",
    ];
    command.extend_from_slice(args);
    run_resolved("agent-session", &command, &CmdOptions::new())
}

#[test]
fn refused_resume_is_readable_without_a_session_or_tmux() {
    let state = tempfile::tempdir().unwrap();
    let failed = run(
        state.path(),
        &["resume", "missing-session", "--format", "json"],
    );
    assert_ne!(failed.code, 0);
    let logs = run(
        state.path(),
        &["logs", "--lifecycle", "missing-session", "--format", "json"],
    );
    assert_eq!(logs.code, 0, "{}", logs.stderr_text());
    let records = logs.stdout_json()["data"]["records"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["operation"], "resume");
    assert_eq!(
        records[0]["result"]["code"],
        failed.stdout_json()["error"]["code"]
    );
    assert_eq!(records[0]["caller"]["kind"], "cli");
    assert!(records[0]["caller"]["binary_version"].is_string());
    assert!(records[0]["caller"]["binary_path"].is_string());
}

#[test]
fn failed_start_records_one_attempt_without_prompt_or_arguments() {
    let state = tempfile::tempdir().unwrap();
    let failed = run(
        state.path(),
        &[
            "start",
            "--agent",
            "codex",
            "--id",
            "failed-start",
            "--prompt",
            "PRIVATE-PROMPT-CANARY",
            "--agent-bin",
            "/missing/provider",
            "--tmux-bin",
            "/missing/tmux",
            "--format",
            "json",
        ],
    );
    assert_ne!(failed.code, 0);
    let logs = run(
        state.path(),
        &["logs", "--lifecycle", "failed-start", "--format", "json"],
    );
    assert_eq!(logs.code, 0, "{}", logs.stderr_text());
    let envelope: Value = logs.stdout_json();
    let records = envelope["data"]["records"].as_array().unwrap();
    assert_eq!(
        records.iter().filter(|r| r["operation"] == "start").count(),
        1
    );
    assert!(!logs.stdout_text().contains("PRIVATE-PROMPT-CANARY"));
    assert!(!logs.stdout_text().contains("/missing/provider"));
}

#[test]
fn refused_delete_account_and_broker_recovery_each_record_one_attempt() {
    let state = tempfile::tempdir().unwrap();
    let proof = state.path().join("proof.json");
    std::fs::write(&proof, "{}").unwrap();
    for (operation, args) in [
        (
            "delete",
            vec!["delete", "missing-session", "--format", "json"],
        ),
        (
            "account-switch",
            vec![
                "account",
                "switch",
                "missing-session",
                "--account",
                "test-account",
                "--format",
                "json",
            ],
        ),
        (
            "broker-adopt",
            vec![
                "broker",
                "adopt",
                "--session",
                "missing-session",
                "--proof-file",
                proof.to_str().unwrap(),
                "--idempotency-key",
                "adopt-attempt",
                "--format",
                "json",
            ],
        ),
        (
            "broker-reconcile",
            vec![
                "broker",
                "reconcile",
                "--session",
                "missing-session",
                "--proof-file",
                proof.to_str().unwrap(),
                "--idempotency-key",
                "reconcile-attempt",
                "--format",
                "json",
            ],
        ),
        (
            "stop",
            vec![
                "broker",
                "stop",
                "--session",
                "missing-session",
                "--format",
                "json",
            ],
        ),
    ] {
        let failed = run(state.path(), &args);
        assert_ne!(failed.code, 0, "{operation}");
        let logs = run(
            state.path(),
            &["logs", "--lifecycle", "missing-session", "--format", "json"],
        );
        assert_eq!(logs.code, 0, "{}", logs.stderr_text());
        let envelope = logs.stdout_json();
        let matching = envelope["data"]["records"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|row| row["operation"] == operation)
            .collect::<Vec<_>>();
        assert_eq!(matching.len(), 1, "{operation}: {envelope}");
        assert_eq!(
            matching[0]["result"]["code"],
            failed.stdout_json()["error"]["code"]
        );
    }
}
