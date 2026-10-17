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

#[test]
fn mailbox_reminder_guidance_exposes_unread_after_a_full_page_of_old_rows() {
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
    let now = jiff::Timestamp::now().as_second();
    super::coordination::rewrite_registry(&state, |registry| {
        registry["messages"] = serde_json::Value::Array((0..51).map(|index| {
            let created = now - 100 + index;
            serde_json::json!({
                "schema_version":"agent-session.message.v1",
                "message_id": if index == 50 { "new-unread".to_string() } else { format!("older-{index:02}") },
                "sender_session_id":"sender", "sender_incarnation":"sender-incarnation",
                "recipient_session_id":"recipient", "recipient_incarnation":"incarnation",
                "state":if index == 50 {"unread"} else {"read"}, "revision":1,
                "reply_to":null, "reply_depth":0,
                "created_at":jiff::Timestamp::from_second(created).unwrap().to_string(), "created_at_epoch":created,
                "expires_at":jiff::Timestamp::from_second(now + 3600).unwrap().to_string(), "expires_at_epoch":now + 3600,
                "body_bytes":7, "body":"fixture"
            })
        }).collect());
    });
    let options = CmdOptions::new()
        .without_ambient_managed_session_env()
        .with_cwd(tmp.path())
        .with_env("AGENT_SESSION_ID", "recipient")
        .with_env(
            "AGENT_SESSION_CAPABILITY_FILE",
            &super::coordination::capability(&state, "recipient"),
        );
    let args = [
        "--state-dir",
        state.to_str().unwrap(),
        "message",
        "inbox",
        "--format",
        "json",
    ];
    let output = run_resolved("agent-session", &args, &options);
    assert_eq!(output.code, 0, "{}", output.stdout_text());
    let data = &output.stdout_json()["data"];
    assert_eq!(data["messages"].as_array().unwrap().len(), 50);
    assert!(
        data["messages"]
            .as_array()
            .unwrap()
            .iter()
            .all(|message| message["state"] == "read")
    );
    assert!(!data["next_cursor"].is_null());
    let guidance = data["guidance"].as_str().unwrap();
    assert!(
        guidance.contains("agent-session message inbox --state unread"),
        "{guidance}"
    );
    assert!(guidance.contains("next_cursor"), "{guidance}");
    let mut unread = args.to_vec();
    unread.extend(["--state", "unread"]);
    let output = run_resolved("agent-session", &unread, &options);
    assert_eq!(output.code, 0, "{}", output.stdout_text());
    assert_eq!(
        output.stdout_json()["data"]["messages"][0]["message_id"],
        "new-unread"
    );
}
