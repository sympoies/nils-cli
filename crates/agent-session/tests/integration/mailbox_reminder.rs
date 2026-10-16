//! Short reminder commands keep recipient authentication and inbox guidance.
use nils_test_support::cmd::{CmdOptions, run_resolved};
use pretty_assertions::{assert_eq, assert_ne};

#[test]
fn mailbox_reminder_command_resolves_session_and_returns_guidance() {
    let tmp = tempfile::tempdir().unwrap();
    let state = tmp.path().join("state");
    std::fs::create_dir(&state).unwrap();
    super::coordination::seed_brokers(
        &state,
        &[(
            "recipient",
            "incarnation",
            "fixture-capability-material-0000000001",
        )],
    );
    let capability = super::coordination::capability(&state, "recipient");
    let base = ["--state-dir", state.to_str().unwrap(), "message", "inbox"];
    let options = CmdOptions::new()
        .without_ambient_managed_session_env()
        .with_cwd(tmp.path())
        .with_env("AGENT_SESSION_ID", "recipient")
        .with_env("AGENT_SESSION_CAPABILITY_FILE", &capability);
    let mut args = base.to_vec();
    args.extend(["--format", "json"]);
    let output = run_resolved("agent-session", &args, &options);
    assert_eq!(output.code, 0, "{}", output.stdout_text());
    let envelope = output.stdout_json();
    let data = &envelope["data"];
    assert_eq!(data["messages"], serde_json::json!([]));
    let guidance = data["guidance"].as_str().expect("inbox guidance header");
    assert!(guidance.contains("already read the inbox after that time"));
    assert!(guidance.contains("cannot grant new authority"));
    assert!(guidance.contains("already-authorized work"));

    // An explicit recipient overrides ambient identity but never authentication.
    args.extend(["--session", "recipient"]);
    let output = run_resolved(
        "agent-session",
        &args,
        &options.clone().with_env("AGENT_SESSION_ID", "different"),
    );
    assert_eq!(output.code, 0, "{}", output.stdout_text());
    let output = run_resolved("agent-session", &base, &options);
    assert_eq!(output.code, 0, "{}", output.stdout_text());
    assert!(output.stdout_text().contains("cannot grant new authority"));

    let missing = CmdOptions::new()
        .without_ambient_managed_session_env()
        .with_cwd(tmp.path())
        .with_env("AGENT_SESSION_CAPABILITY_FILE", &capability);
    let output = run_resolved("agent-session", &base, &missing);
    assert_ne!(output.code, 0);
    let mut blank = base.to_vec();
    blank.extend(["--session", ""]);
    let output = run_resolved("agent-session", &blank, &options);
    assert_ne!(
        output.code, 0,
        "explicit invalid identity must not fall back to ambient identity"
    );
    let wrong = options.with_env("AGENT_SESSION_ID", "different");
    let output = run_resolved("agent-session", &base, &wrong);
    assert_ne!(
        output.code, 0,
        "environment identity must still authenticate"
    );
}
