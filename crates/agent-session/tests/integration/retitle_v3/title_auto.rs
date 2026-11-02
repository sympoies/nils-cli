//! The CLI's mode change gates and then resumes real automatic title inference.
use super::*;
use nils_test_support::cmd::{CmdOptions, run_resolved};
use pretty_assertions::{assert_eq, assert_ne};

#[test]
fn title_auto_resumes_generated_topics_after_a_cli_pin() {
    let fixture = Fixture::new();
    let record: Value = serde_json::from_slice(&fs::read(&fixture.record_path).unwrap()).unwrap();
    super::super::coordination::seed_brokers(
        &fixture.state_dir,
        &[(
            SESSION_ID,
            INCARNATION,
            "synthetic-title-writer-capability-material",
        )],
    );
    write_private_json(&fixture.record_path, &record);
    let capability = super::super::coordination::capability(&fixture.state_dir, SESSION_ID);
    let command = |args: &[&str]| {
        let mut argv = vec!["--state-dir", fixture.state_dir.to_str().unwrap(), "title"];
        argv.extend_from_slice(args);
        argv.extend(["--capability-file", &capability, "--format", "json"]);
        let output = run_resolved(
            "agent-session",
            &argv,
            &CmdOptions::new()
                .with_cwd(&fixture.root)
                .without_ambient_managed_session_env(),
        );
        assert_eq!(
            output.code,
            0,
            "{} {}",
            output.stdout_text(),
            output.stderr_text()
        );
    };
    command(&[
        "set",
        SESSION_ID,
        "Pinned review topic",
        "--if-revision",
        "0",
    ]);
    seed_automatic_activity(&fixture.state_dir, "title-turn", 1);
    let _server = ServeProcess::spawn(&fixture);
    let request = |key, revision| {
        fixture.request(
            "POST",
            &format!("/sessions/{SESSION_ID}/retitle-v3"),
            Some(TOKEN),
            Some(&automatic_request(key, revision, 0, 1, "title-turn")),
        )
    };
    let pinned = request("automatic-while-pinned", 1);
    assert_eq!(pinned.status, 409, "{}", pinned.body);
    assert_eq!(pinned.body["error"]["code"], "title-mode-pinned");
    assert!(!fixture.provider_calls.exists());
    command(&["auto", SESSION_ID, "--if-revision", "1"]);
    let admitted = request("automatic-after-auto", 2);
    assert!(matches!(admitted.status, 200 | 202), "{}", admitted.body);
    let terminal = poll_terminal(
        &fixture,
        admitted.body["data"]["retitle"]["operation_hash"]
            .as_str()
            .unwrap(),
    );
    assert_eq!(
        terminal.body["data"]["retitle"]["outcome"], "committed",
        "{}",
        terminal.body
    );
    let record: Value = serde_json::from_slice(&fs::read(&fixture.record_path).unwrap()).unwrap();
    assert_ne!(record["title"], "Pinned review topic");
    assert_eq!(record["title_state"]["topic_source"], "auto");
    assert_eq!(record["title_mode"], "auto");
}
