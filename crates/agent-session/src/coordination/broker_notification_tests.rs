use super::*;
use pretty_assertions::assert_eq;

fn fixture(receipts: Value, messages: Value) -> (tempfile::TempDir, CliContext) {
    let dir = tempfile::tempdir().unwrap();
    let context = CliContext {
        state_dir: dir.path().to_path_buf(),
        host: None,
    };
    let record: SessionRecord = serde_json::from_value(json!({
        "schema_version": crate::SESSION_DOCUMENT_VERSION,
        "id": "session", "agent": "codex", "mode": "interactive",
        "cwd": "/srv/workspace", "tmux_session": "fixture-session",
        "created_at": "2030-01-01T00:00:00Z", "updated_at": "2030-01-01T00:00:00Z",
        "runtime": {
            "kind": "tmux", "tmux_session": "fixture-session", "generation": 1,
            "started_at": "2030-01-01T00:00:00Z", "launch_id": "current"
        }
    }))
    .unwrap();
    crate::write_session_record(&context, &record).unwrap();
    test_support::seed_live_broker(&context, "session", "current", json!({}));
    let path = registry_path(&context);
    let mut registry: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    registry["notifications"] = receipts;
    registry["messages"] = messages;
    write_atomic(
        &path,
        &serde_json::to_vec(&registry).unwrap(),
        SECRET_FILE_MODE,
    )
    .unwrap();
    (dir, context)
}

fn registry_path(context: &CliContext) -> PathBuf {
    context.state_dir.join("coordination/registry.json")
}

fn receipt(state: &str, incarnation: &str) -> Value {
    json!({
        "schema_version": "agent-session.notification-generation.v1",
        "target_session_id": "session", "target_incarnation": incarnation,
        "state": state, "generation": 2, "attempted_generation": 1,
        "notified_generation": 0, "queued_at_epoch": 100,
        "attempted_queued_at_epoch": 80, "attempted_at_epoch": 90,
        "updated_at_epoch": 101, "next_attempt_at_epoch": 120,
        "last_reason": "notification-pending"
    })
}

fn message(incarnation: &str, state: &str, expires: i64) -> Value {
    json!({
        "schema_version": "agent-session.message.v1", "message_id": "synthetic-message",
        "sender_session_id": "remote:synthetic-sender", "sender_incarnation": "remote-incarnation",
        "recipient_session_id": "session", "recipient_incarnation": incarnation,
        "state": state, "revision": 1, "reply_to": null, "reply_depth": 0,
        "created_at": "1970-01-01T00:03:20Z", "created_at_epoch": 200,
        "remote_created_at_epoch": 9000000000_i64,
        "expires_at": "2100-01-01T00:00:00Z", "expires_at_epoch": expires,
        "body_bytes": 25, "body": "synthetic-private-content"
    })
}

fn inspect(context: &CliContext) -> Value {
    let before = fs::read(registry_path(context)).unwrap();
    let result = status(
        context,
        BrokerStatusArgs {
            session: "session".into(),
            capability_file: None,
            authenticated: false,
            format: nils_common::cli_contract::OutputFormat::Json,
        },
    )
    .unwrap();
    assert_eq!(fs::read(registry_path(context)).unwrap(), before);
    result
}

#[test]
fn inspection_reports_absence_and_selects_only_the_current_incarnation() {
    for receipts in [json!({}), json!({"old": receipt("queued", "old")})] {
        let (_dir, context) = fixture(receipts, json!([]));
        let value = inspect(&context);
        assert_eq!(value["session_incarnation"], "current");
        assert!(value.get("notification").unwrap().is_null());
    }
}

#[test]
fn inspection_distinguishes_receipt_states_and_attempt_timestamps() {
    for state in [
        "queued",
        "attempting",
        "attempt_unknown",
        "prompt_submitted",
        "undeliverable",
    ] {
        let mut stored = receipt(state, "current");
        // Keep unknown outcomes on the attempted generation; newer unknown
        // generations are separately covered by the normalization regression.
        if state == "attempt_unknown" {
            stored["attempted_generation"] = json!(2);
        }
        if state == "prompt_submitted" {
            stored["notified_generation"] = json!(2);
        }
        let (_dir, context) = fixture(
            json!({"current": stored}),
            json!([message("current", "unread", i64::MAX)]),
        );
        let before = now_epoch();
        let value = inspect(&context);
        let notification = &value["notification"];
        assert_eq!(notification["state"], state);
        assert_eq!(notification["generation"], 2);
        assert_eq!(
            notification["attempted_generation"],
            if state == "attempt_unknown" { 2 } else { 1 }
        );
        assert_eq!(notification["attempted_queued_at_epoch"], 80);
        assert_eq!(notification["attempted_at_epoch"], 90);
        assert_eq!(notification["updated_at_epoch"], 101);
        assert_eq!(notification["next_attempt_at_epoch"], 120);
        assert_eq!(
            notification["queued_at_epoch"],
            if state == "queued" { 200 } else { 100 }
        );
        assert_eq!(notification["controller_available"], Value::Null);
        assert_eq!(notification["live_unread_count"], 1);
        assert_eq!(notification["resolved"], state == "prompt_submitted");
        assert!(notification["observed_at_epoch"].as_i64().unwrap() >= before);
        assert!(notification["observed_at_epoch"].as_i64().unwrap() <= now_epoch());
        assert!(!value.to_string().contains("synthetic-private-content"));
        assert!(!value.to_string().contains("remote:synthetic-sender"));
    }
}

#[test]
fn inspection_recovers_local_queue_time_without_changing_the_attempt_or_registry() {
    let (_dir, context) = fixture(
        json!({"current": receipt("attempt_unknown", "current")}),
        json!([message("current", "unread", i64::MAX)]),
    );
    let value = inspect(&context);
    let notification = &value["notification"];
    assert_eq!(notification["state"], "queued");
    assert_eq!(notification["generation"], 2);
    assert_eq!(notification["attempted_generation"], 1);
    assert_eq!(notification["queued_at_epoch"], 200);
    assert_eq!(notification["attempted_queued_at_epoch"], 80);
    assert_eq!(notification["source"]["registry"], "canonical-local");
    assert_eq!(notification["source"]["normalization_applied"], true);
    assert_eq!(
        notification["source"]["queued_at"],
        "canonical-unread-admission"
    );
}

#[test]
fn inspection_counts_remote_unread_mail_but_excludes_other_incarnations_and_expiry() {
    let (_dir, context) = fixture(
        json!({"current": receipt("attempting", "current")}),
        json!([
            message("current", "unread", i64::MAX),
            message("old", "unread", i64::MAX),
            message("current", "read", i64::MAX),
            message("current", "unread", 1)
        ]),
    );
    let notification = inspect(&context)["notification"].clone();
    assert_eq!(notification["live_unread_count"], 1);
    assert_eq!(notification["resolved"], false);
}

#[test]
fn inspection_distinguishes_drained_mail_from_submitted_prompt_and_sanitizes_legacy_values() {
    let mut stored = receipt("queued", "current");
    stored["last_reason"] = json!("synthetic-private-content");
    for field in [
        "queued_at_epoch",
        "attempted_queued_at_epoch",
        "attempted_at_epoch",
        "next_attempt_at_epoch",
    ] {
        stored[field] = json!(0);
    }
    let (_dir, context) = fixture(json!({"current": stored}), json!([]));
    let value = inspect(&context);
    let notification = &value["notification"];
    assert_eq!(notification["state"], "queued");
    assert_eq!(notification["notified_generation"], 0);
    assert_eq!(notification["live_unread_count"], 0);
    assert_eq!(notification["resolved"], true);
    assert_eq!(notification["last_reason"], "notification-state-invalid");
    for field in [
        "queued_at_epoch",
        "attempted_queued_at_epoch",
        "attempted_at_epoch",
        "next_attempt_at_epoch",
    ] {
        assert_eq!(notification[field], Value::Null);
    }
    let text = value.to_string();
    assert!(text.len() < 2048);
    assert!(!text.contains("synthetic-private-content"));
    assert!(!text.contains("capability_digest"));
    assert!(!text.contains("capability_file"));
}
