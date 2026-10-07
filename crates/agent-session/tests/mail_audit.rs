use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

use pretty_assertions::assert_eq;
use serde_json::{Value, json};

fn write_private(path: &std::path::Path, value: &Value) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}

fn audit(root: &std::path::Path, extra: &[&str]) -> Value {
    let mut command = Command::new(nils_test_support::bin::resolve("agent-session"));
    nils_test_support::cmd::strip_ambient_managed_session_env(&mut command);
    let output = command
        .args([
            "--state-dir",
            root.to_str().unwrap(),
            "message",
            "audit",
            "--format",
            "json",
        ])
        .args(extra)
        .env("AGENT_SESSION_MACHINE", "destination")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        format!(
            "stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        value["schema_version"],
        "cli.agent-session.message-audit.v1"
    );
    value["data"].clone()
}

#[test]
fn audit_fixture_lists_three_anomaly_classes_without_bodies_or_mutation() {
    let temp = tempfile::TempDir::new().unwrap();
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/coordination/mail-audit-v1.json")).unwrap();
    let registry = temp.path().join("coordination/registry.json");
    let journal = temp.path().join("coordination/federation-journal.json");
    write_private(&registry, &fixture["registry"]);
    write_private(&journal, &fixture["journal"]);
    write_private(
        &temp.path().join("sessions/stopped/session.json"),
        &json!({
            "schema_version":"agent-session.session.v1", "id":"stopped", "agent":"codex", "mode":"interactive",
            "title":null, "cwd":".", "tmux_session":"fixture", "prompt_file":null, "log_file":null,
            "created_at":"2030-01-01T00:00:00Z", "updated_at":"2030-01-01T00:00:00Z",
            "runtime":{"kind":"tmux", "tmux_session":"fixture", "generation":1, "started_at":"2030-01-01T00:00:00Z", "launch_id":"current-incarnation"}
        }),
    );
    let before_registry = fs::read(&registry).unwrap();
    let before_journal = fs::read(&journal).unwrap();
    let page = audit(temp.path(), &[]);
    assert_eq!(page["schema_version"], "agent-session.mail-audit.v1");
    assert_eq!(page["older_than_seconds"], 300);
    let records = page["records"].as_array().unwrap();
    let row = |id: &str| records.iter().find(|r| r["message_id"] == id).unwrap();
    assert!(
        row("overdue")["anomalies"]
            .as_array()
            .unwrap()
            .contains(&json!("overdue-unread"))
    );
    assert!(
        row("accepted-then-stopped")["anomalies"]
            .as_array()
            .unwrap()
            .contains(&json!("recipient-stopped"))
    );
    assert!(
        row("old-incarnation")["anomalies"]
            .as_array()
            .unwrap()
            .contains(&json!("incarnation-mismatch"))
    );
    assert!(
        row("missing")["anomalies"]
            .as_array()
            .unwrap()
            .contains(&json!("recipient-missing"))
    );
    assert!(
        row("queued")["anomalies"]
            .as_array()
            .unwrap()
            .contains(&json!("queued-overdue"))
    );
    assert!(
        row("failed-delivery")["anomalies"]
            .as_array()
            .unwrap()
            .contains(&json!("rejected"))
    );
    assert_eq!(row("failed-delivery")["sent_at"], Value::Null);
    assert_eq!(row("failed-delivery")["last_attempt_at"], Value::Null);
    assert_eq!(row("queued")["recipient_status"]["runtime"], "unknown");
    assert_eq!(row("overdue")["notification"]["state"], "prompt_submitted");
    assert!(row("overdue")["end_to_end_latency_seconds"].is_number());
    for canary in [
        "BODY_CANARY",
        "TOKEN_CANARY",
        "HASH_CANARY",
        "PATH_CANARY",
        "REASON_CANARY",
    ] {
        assert!(!page.to_string().contains(canary), "{canary}");
    }
    assert_eq!(fs::read(&registry).unwrap(), before_registry);
    assert_eq!(fs::read(&journal).unwrap(), before_journal);
    let first = audit(temp.path(), &["--limit", "1"]);
    let mut cursor = first["next_cursor"].as_str().unwrap().to_owned();
    let mut paged = first["records"].as_array().unwrap().clone();
    loop {
        let next = audit(temp.path(), &["--limit", "1", "--cursor", &cursor]);
        paged.extend(next["records"].as_array().unwrap().clone());
        let Some(next_cursor) = next["next_cursor"].as_str() else {
            break;
        };
        cursor = next_cursor.to_owned();
    }
    assert_eq!(
        paged.iter().map(|r| &r["message_id"]).collect::<Vec<_>>(),
        records.iter().map(|r| &r["message_id"]).collect::<Vec<_>>()
    );
}

#[test]
fn audit_cli_preserves_parse_errors_and_reports_semantic_query_errors() {
    let temp = tempfile::TempDir::new().unwrap();
    for (args, expected) in [
        (vec!["--older-than", "nope"], "parse-error"),
        (vec!["--limit", "0"], "mail-audit-query-invalid"),
        (vec!["--cursor", "invalid"], "mail-audit-query-invalid"),
    ] {
        let mut command = Command::new(nils_test_support::bin::resolve("agent-session"));
        nils_test_support::cmd::strip_ambient_managed_session_env(&mut command);
        let output = command
            .args([
                "--state-dir",
                temp.path().to_str().unwrap(),
                "message",
                "audit",
                "--format",
                "json",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(!output.status.success());
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            value["schema_version"],
            "cli.agent-session.message-audit.v1"
        );
        assert_eq!(value["error"]["code"], expected);
    }
}
