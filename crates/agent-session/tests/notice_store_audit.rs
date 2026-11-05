//! Public maintenance boundary, always isolated from the operator's state.
use nils_test_support::cmd::{CmdOptions, run_resolved};
use pretty_assertions::assert_eq;

#[test]
fn audit_reports_verified_state_and_refuses_missing_committed_index() {
    let root = tempfile::tempdir().unwrap();
    let options = CmdOptions::new().with_env_remove_prefix("AGENT_SESSION_");
    let args = [
        "--state-dir",
        root.path().to_str().unwrap(),
        "notice-store",
        "audit",
        "--format",
        "json",
    ];
    let output = run_resolved("agent-session", &args, &options);
    assert_eq!(output.code, 0, "{}", output.stderr_text());
    let body = output.stdout_json();
    assert_eq!(
        body["schema_version"],
        "cli.agent-session.notice-store.audit.v1"
    );
    assert_eq!(body["data"]["verified"], true);
    let parent = root.path().join("history/claude-notices");
    let checkpoint = std::fs::read(parent.join("checkpoint.json")).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&checkpoint).unwrap();
    let index = parent.join(value["index"].as_str().unwrap());
    std::fs::remove_file(&index).unwrap();
    let output = run_resolved("agent-session", &args, &options);
    assert_eq!(output.code, 1);
    assert_eq!(
        output.stdout_json()["error"]["code"],
        "notice-store-audit-failed"
    );
    assert_eq!(
        std::fs::read(parent.join("checkpoint.json")).unwrap(),
        checkpoint
    );
    assert!(
        !index.exists(),
        "audit must never recreate missing committed state"
    );
}

#[test]
fn invalid_audit_budget_refuses_before_creating_state() {
    let root = tempfile::tempdir().unwrap();
    let state = root.path().join("absent-state");
    let output = run_resolved(
        "agent-session",
        &[
            "--state-dir",
            state.to_str().unwrap(),
            "notice-store",
            "audit",
            "--timeout-seconds",
            "0",
            "--format",
            "json",
        ],
        &CmdOptions::new().with_env_remove_prefix("AGENT_SESSION_"),
    );
    assert_eq!(output.code, 64);
    assert_eq!(
        output.stdout_json()["schema_version"],
        "cli.agent-session.notice-store.audit.v1"
    );
    assert!(!state.exists());
}

#[test]
fn audit_context_errors_keep_the_leaf_envelope() {
    let root = tempfile::tempdir().unwrap();
    let state = root.path().join("absent-state");
    let output = run_resolved(
        "agent-session",
        &[
            "--state-dir",
            state.to_str().unwrap(),
            "--host",
            "invalid host",
            "notice-store",
            "audit",
            "--format",
            "json",
        ],
        &CmdOptions::new().with_env_remove_prefix("AGENT_SESSION_"),
    );
    assert_eq!(output.code, 64);
    assert_eq!(output.stdout_json()["error"]["code"], "invalid-host");
    assert_eq!(
        output.stdout_json()["schema_version"],
        "cli.agent-session.notice-store.audit.v1"
    );
    assert!(!state.exists());
}
